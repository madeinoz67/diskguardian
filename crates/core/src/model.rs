use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Index into [`Tree::nodes`].
pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Directory,
    File,
    Symlink,
    /// Could not be read (permission denied, I/O error). Carries the reason in `note`.
    Restricted,
}

impl NodeKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "dir",
            Self::File => "file",
            Self::Symlink => "symlink",
            Self::Restricted => "restricted",
        }
    }
}

/// One object on disk.
///
/// `own_bytes` is space attributable to this node itself: a file's allocation, or
/// for a directory, its own metadata blocks plus any entries that could not be
/// turned into child nodes. `total_bytes` is `own_bytes` plus the totals of all
/// children, so the tree never sums a parent smaller than its children.
///
/// Only the name is stored. A `PathBuf` per node duplicates the whole parent path on
/// every entry, which on a home directory is gigabytes of repeated strings; paths are
/// reconstructed on demand by [`Tree::path_of`], which is called a handful of times per
/// user action rather than millions of times per scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub own_bytes: u64,
    pub total_bytes: u64,
    pub file_count: u64,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    /// True when something below this node could not be measured.
    pub partial: bool,
    /// True for a hard link whose inode was already counted elsewhere.
    pub link_skipped: bool,
    /// Gone from disk since the scan. Still listed so the user can see what they did,
    /// struck through and worth zero bytes.
    pub missing: bool,
    /// Newest modification time anywhere in this subtree, in Unix seconds.
    ///
    /// A directory's own mtime only moves when its entry list changes, so it says nothing
    /// about whether the *contents* are stale. The newest descendant is what an age rule
    /// ("untouched for 90 days") actually means.
    pub newest_modified: Option<u64>,
    pub note: Option<String>,
}

impl Node {
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// A full scan result. Built once per volume, queried many times by the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    pub scanned_root: PathBuf,
    pub duration_ms: u64,
    pub errors: u64,
}

impl Tree {
    #[must_use]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.nodes[self.root].total_bytes
    }

    #[must_use]
    pub fn file_count(&self) -> u64 {
        self.nodes[self.root].file_count
    }

    /// Mark nodes as gone from disk: struck through, worth nothing, still listed, with
    /// every ancestor's totals reduced accordingly.
    ///
    /// Removing them outright was the first design and it made a deletion look like nothing
    /// had happened. DaisyDisk keeps the item visible and strikes it out so the user can see
    /// what they just did; the next scan is what clears it.
    pub fn mark_missing(&mut self, ids: &[NodeId]) {
        let mut touched: Vec<NodeId> = Vec::new();

        for &id in ids {
            if id >= self.nodes.len() || id == self.root {
                continue;
            }
            self.nodes[id].missing = true;
            self.nodes[id].own_bytes = 0;
            self.nodes[id].total_bytes = 0;
            self.nodes[id].file_count = 0;
            if let Some(parent) = self.nodes[id].parent {
                touched.push(parent);
            }
        }

        for parent in touched {
            self.recompute_from(parent);
        }
    }

    /// Recompute one node's totals from its children, then walk up to the root.
    fn recompute_from(&mut self, start: NodeId) {
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if self.nodes[id].missing {
                // A missing node contributes nothing, whatever still hangs off it.
                self.nodes[id].total_bytes = 0;
                self.nodes[id].file_count = 0;
                cursor = self.nodes[id].parent;
                continue;
            }
            let child_total: u64 = self.nodes[id]
                .children
                .iter()
                .map(|&c| self.nodes[c].total_bytes)
                .sum();
            let child_files: u64 = self.nodes[id]
                .children
                .iter()
                .map(|&c| self.nodes[c].file_count)
                .sum();
            self.nodes[id].total_bytes = self.nodes[id].own_bytes.saturating_add(child_total);
            self.nodes[id].file_count = child_files;
            cursor = self.nodes[id].parent;
        }
    }

    /// Walk from a node up to the scan root.
    #[must_use]
    pub fn ancestors(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cur = self.nodes[id].parent;
        while let Some(p) = cur {
            out.push(p);
            cur = self.nodes[p].parent;
        }
        out
    }

    /// Rebuild an absolute path from the name chain. The root's own name is the last
    /// component of `scanned_root`, so it is not re-appended.
    #[must_use]
    pub fn path_of(&self, id: NodeId) -> PathBuf {
        if id == self.root {
            return self.scanned_root.clone();
        }
        let mut names = Vec::new();
        let mut cur = Some(id);
        while let Some(node_id) = cur {
            if node_id == self.root {
                break;
            }
            names.push(self.nodes[node_id].name.as_str());
            cur = self.nodes[node_id].parent;
        }
        let mut path = self.scanned_root.clone();
        for name in names.iter().rev() {
            path.push(name);
        }
        path
    }

    /// True when nothing in this subtree has been written since `cutoff` (Unix seconds).
    #[must_use]
    pub fn older_than(&self, id: NodeId, cutoff: u64) -> bool {
        self.nodes[id].newest_modified.is_some_and(|m| m < cutoff)
    }

    /// How many levels below the scan root this node sits.
    #[must_use]
    pub fn depth_of(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut cur = self.nodes[id].parent;
        while let Some(p) = cur {
            depth += 1;
            cur = self.nodes[p].parent;
        }
        depth
    }

    /// The topmost `Restricted` nodes: the ones with no `Restricted` ancestor.
    ///
    /// These are exactly the paths an administrator pass should be asked to measure.
    /// Descendants of a Restricted node are invisible (the walk stopped at the
    /// ancestor), so requesting them would be requesting nothing twice. A fresh walk
    /// cannot nest Restricted nodes — they have no children — but a grafted subtree can
    /// carry its own, so "topmost" is enforced rather than assumed. Nodes the tree
    /// believes are gone (`missing`) are not worth a prompt.
    #[must_use]
    pub fn restricted_roots(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for (i, node) in self.nodes.iter().enumerate() {
            if node.kind != NodeKind::Restricted || node.missing {
                continue;
            }
            let mut has_restricted_ancestor = false;
            let mut cur = node.parent;
            while let Some(p) = cur {
                if self.nodes[p].kind == NodeKind::Restricted {
                    has_restricted_ancestor = true;
                    break;
                }
                cur = self.nodes[p].parent;
            }
            if !has_restricted_ancestor {
                out.push(i);
            }
        }
        out
    }

    /// Splice a separately-scanned subtree in place of the node at `id`.
    ///
    /// This is how an administrator measurement pass (which rescans a restricted path
    /// as its own root, in an elevated process) rejoins the main tree. On success the
    /// node at `id` takes the subtree root's `kind`, `own_bytes`, `file_count`, `note`
    /// and `partial`, the subtree root's children are copied in with all indices
    /// remapped, and every ancestor's totals and `partial` flags are recomputed.
    ///
    /// The subtree's `scanned_root` must equal the path of the node it replaces,
    /// compared as strings, no canonicalisation: both strings are produced by this
    /// program, and canonicalising here would re-import the `/var` vs `/private/var`
    /// mismatch that bit C17.
    pub fn graft(&mut self, id: NodeId, subtree: &Tree) -> Result<(), GraftError> {
        let node_path = self.path_of(id);
        if subtree.scanned_root != node_path {
            return Err(GraftError::RootMismatch {
                subtree_root: subtree.scanned_root.clone(),
                node_path,
            });
        }
        if !is_arena_coherent(subtree) {
            return Err(GraftError::MalformedSubtree);
        }

        // Copy every non-root node of the subtree into the arena, remapping ids. The
        // map is filled in one pass (parents precede children in a coherent tree) and
        // consumed in a second, because a node's children are always copied after it.
        let sub_root = subtree.root;
        let offset = self.nodes.len();
        let mut map: Vec<Option<NodeId>> = vec![None; subtree.nodes.len()];
        let mut next = offset;
        for (i, node) in subtree.nodes.iter().enumerate() {
            if i == sub_root {
                continue;
            }
            map[i] = Some(next);
            let mut copy = node.clone();
            copy.parent = match node.parent {
                Some(p) if p == sub_root => Some(id),
                Some(p) => map[p],
                None => None,
            };
            self.nodes.push(copy);
            next += 1;
        }
        let remap = |child: &NodeId| -> Option<NodeId> { map[*child] };

        let sub_root_node = &subtree.nodes[sub_root];
        let spliced: Vec<NodeId> = sub_root_node.children.iter().filter_map(remap).collect();
        let target = &mut self.nodes[id];
        target.kind = sub_root_node.kind;
        target.own_bytes = sub_root_node.own_bytes;
        target.file_count = sub_root_node.file_count;
        target.note.clone_from(&sub_root_node.note);
        // The subtree's own walk already encoded anything it could not read in this
        // flag; `recompute_partial_from` will OR it into every ancestor.
        target.partial = sub_root_node.partial;
        target.children = spliced;

        self.recompute_from(id);
        if let Some(parent) = self.nodes[id].parent {
            self.recompute_partial_from(parent);
        }
        Ok(())
    }

    /// Recompute `partial` for one node from its children, then walk up to the root.
    ///
    /// A mirror of [`Tree::recompute_from`], needed because `partial` only ever gets
    /// set during a walk, never cleared: a graft that resolves every restricted node
    /// below must be able to un-flag the ancestors. A node is partial when it is
    /// itself restricted or any child is.
    ///
    /// Known edge: a node whose *own* iteration failed mid-`readdir` (not a child's
    /// failure) also sets `partial` during a walk, and that local cause is not stored
    /// separately, so this recompute can clear it. In practice `max_entries` is never
    /// set by the app and mid-iteration failures are rarer than permission failures,
    /// which are always child flags; the alternative is a second persisted flag, which
    /// this slice does not buy.
    fn recompute_partial_from(&mut self, start: NodeId) {
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            let node = &self.nodes[id];
            let partial = node.kind == NodeKind::Restricted
                || node.children.iter().any(|&c| self.nodes[c].partial);
            cursor = node.parent;
            self.nodes[id].partial = partial;
        }
    }
}

/// Whether `tree` is a well-formed arena tree: every id in range, parents before
/// children, the root parentless, no child pointing at the root, and parent/child
/// links agreeing in both directions. A tampered results file must fail this check
/// rather than corrupt the arena it is grafted into.
fn is_arena_coherent(tree: &Tree) -> bool {
    for (i, node) in tree.nodes.iter().enumerate() {
        match node.parent {
            None if i != tree.root => return false,
            Some(_) if i == tree.root => return false,
            Some(p) => {
                if p >= tree.nodes.len() || p == i {
                    return false;
                }
                if !tree.nodes[p].children.contains(&i) {
                    return false;
                }
            }
            None => {}
        }
        if node.children.len()
            != node
                .children
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        {
            return false;
        }
        for &child in &node.children {
            if child >= tree.nodes.len() || child == tree.root {
                return false;
            }
            if tree.nodes[child].parent != Some(i) {
                return false;
            }
        }
    }
    tree.root < tree.nodes.len()
}

/// Why a [`Tree::graft`] was refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GraftError {
    #[error("graft refused: subtree was scanned from {subtree_root}, but the node is {node_path}")]
    RootMismatch {
        subtree_root: PathBuf,
        node_path: PathBuf,
    },
    #[error("graft refused: the subtree is not a well-formed tree")]
    MalformedSubtree,
}
