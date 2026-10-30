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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
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
}
