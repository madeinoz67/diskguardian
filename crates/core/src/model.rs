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

    /// Rewire every child's `parent` pointer. Called once after a scan finishes,
    /// because children are built before the parent exists.
    pub fn link_parents(nodes: &mut [Node]) {
        for i in 0..nodes.len() {
            let children = std::mem::take(&mut nodes[i].children);
            for &c in &children {
                nodes[c].parent = Some(i);
            }
            nodes[i].children = children;
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
}
