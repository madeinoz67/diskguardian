//! The walker.
//!
//! Physical-size accounting is the point of this module. A file's size on disk is
//! `st_blocks * 512`, not `st_size`: sparse files occupy less, and the last block
//! is rounded up. Hard links are counted once per inode; the extra names appear as
//! zero-size links so the tree still shows them.
//!
//! Directories are walked in parallel with rayon. Symlinks are never followed, so
//! a symlink loop cannot hang a scan and a link costs only its own metadata.

use crate::model::{Node, NodeId, NodeKind, Tree};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read {path}: {source}")]
    Root {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Optional cap on files visited, for a fast path over enormous volumes.
    pub max_entries: Option<u64>,
}

/// Scan `root` and return the whole tree. The root itself may be a file.
pub fn scan(root: &Path, opts: &ScanOptions) -> Result<Tree, ScanError> {
    let started = Instant::now();
    let meta = fs::symlink_metadata(root).map_err(|source| ScanError::Root {
        path: root.to_path_buf(),
        source,
    })?;

    let builder = Builder {
        seen: Mutex::new(HashSet::new()),
        max_entries: opts.max_entries,
        visited: Mutex::new(0u64),
    };

    let name = root_name(root);
    let sub = if meta.is_dir() {
        builder.walk_dir(root, name)
    } else {
        builder.walk_file(root, name, &meta)
    };

    let mut nodes = sub.nodes;
    Tree::link_parents(&mut nodes);

    Ok(Tree {
        nodes,
        root: sub.root,
        scanned_root: root.to_path_buf(),
        duration_ms: started.elapsed().as_millis() as u64,
        errors: sub.errors,
    })
}

struct Subtree {
    nodes: Vec<Node>,
    root: NodeId,
    errors: u64,
}

struct Builder {
    seen: Mutex<HashSet<(u64, u64)>>,
    max_entries: Option<u64>,
    visited: Mutex<u64>,
}

impl Builder {
    fn walk_dir(&self, path: &Path, name: String) -> Subtree {
        let mut nodes: Vec<Node> = Vec::new();
        let mut child_ids: Vec<NodeId> = Vec::new();
        let mut own_bytes = 0u64;
        let mut errors = 0u64;
        let mut partial = false;

        let read_dir = match fs::read_dir(path) {
            Ok(rd) => rd,
            Err(err) => {
                let mut node = Node {
                    name,
                    path: path.to_path_buf(),
                    kind: NodeKind::Restricted,
                    own_bytes: 0,
                    total_bytes: 0,
                    file_count: 0,
                    children: Vec::new(),
                    parent: None,
                    partial: true,
                    link_skipped: false,
                    note: Some(err.to_string()),
                };
                // A directory we cannot enter still occupies blocks of its own.
                if let Ok(meta) = fs::symlink_metadata(path) {
                    node.own_bytes = allocation(&meta);
                    node.total_bytes = node.own_bytes;
                }
                return Subtree {
                    nodes: vec![node],
                    root: 0,
                    errors: 1,
                };
            }
        };

        let mut dirs: Vec<fs::DirEntry> = Vec::new();
        let mut files: Vec<fs::DirEntry> = Vec::new();

        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    errors += 1;
                    partial = true;
                    continue;
                }
            };
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => dirs.push(entry),
                Ok(_) => files.push(entry),
                Err(_) => {
                    errors += 1;
                    partial = true;
                }
            }
        }

        // Deterministic order, so two scans of an unchanged tree agree.
        dirs.sort_by_key(fs::DirEntry::file_name);
        files.sort_by_key(fs::DirEntry::file_name);

        let subtrees: Vec<Subtree> = dirs
            .par_iter()
            .map(|entry| self.walk_dir(&entry.path(), entry_name(entry)))
            .collect();

        for sub in subtrees {
            let offset = nodes.len();
            errors += sub.errors;
            if sub.nodes[sub.root].partial {
                partial = true;
            }
            nodes.extend(sub.nodes);
            for node in &mut nodes[offset..] {
                for child in &mut node.children {
                    *child += offset;
                }
                if let Some(parent) = node.parent.as_mut() {
                    *parent += offset;
                }
            }
            child_ids.push(offset + sub.root);
        }

        for entry in &files {
            // `DirEntry::metadata` does not traverse symlinks, which is exactly what we
            // want: a link costs only its own metadata, never its target.
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    errors += 1;
                    partial = true;
                    continue;
                }
            };
            if self.at_limit() {
                partial = true;
                break;
            }
            let sub = self.leaf_node(entry.path(), entry_name(entry), &meta);
            let idx = nodes.len();
            nodes.push(sub);
            child_ids.push(idx);
        }

        // The directory's own metadata blocks are real occupied space.
        if let Ok(meta) = fs::metadata(path) {
            own_bytes += allocation(&meta);
        }

        let child_total: u64 = child_ids.iter().map(|&c| nodes[c].total_bytes).sum();
        let total_bytes = own_bytes.saturating_add(child_total);

        // Leaves count 1 each, so summing the children's counts is the whole subtree:
        // adding `direct_files` on top would count this directory's files twice.
        let files_in_subtree: u64 = child_ids.iter().map(|&c| nodes[c].file_count).sum();

        let idx = nodes.len();
        nodes.push(Node {
            name,
            path: path.to_path_buf(),
            kind: NodeKind::Directory,
            own_bytes,
            total_bytes,
            file_count: files_in_subtree,
            children: child_ids,
            parent: None,
            partial,
            link_skipped: false,
            note: None,
        });

        Subtree {
            nodes,
            root: idx,
            errors,
        }
    }

    fn walk_file(&self, path: &Path, name: String, meta: &fs::Metadata) -> Subtree {
        Subtree {
            nodes: vec![self.leaf_node(path.to_path_buf(), name, meta)],
            root: 0,
            errors: 0,
        }
    }

    fn leaf_node(&self, path: PathBuf, name: String, meta: &fs::Metadata) -> Node {
        let allocation = allocation(meta);
        let is_link = meta.file_type().is_symlink();
        let duplicated = is_multilink(meta) && !self.claim_inode(meta);

        let (own_bytes, note) = if duplicated {
            (
                0,
                Some(format!(
                    "hard link to an inode already counted ({allocation} bytes)"
                )),
            )
        } else {
            (allocation, None)
        };

        Node {
            name,
            path,
            kind: if is_link {
                NodeKind::Symlink
            } else {
                NodeKind::File
            },
            own_bytes,
            total_bytes: own_bytes,
            file_count: 1,
            children: Vec::new(),
            parent: None,
            partial: false,
            link_skipped: duplicated,
            note,
        }
    }

    /// Returns true if this inode has not been seen before (and records it).
    fn claim_inode(&self, meta: &fs::Metadata) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let key = (meta.dev(), meta.ino());
            let mut seen = self
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            seen.insert(key)
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            true
        }
    }

    fn at_limit(&self) -> bool {
        let Some(limit) = self.max_entries else {
            return false;
        };
        let mut visited = self
            .visited
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *visited >= limit {
            return true;
        }
        *visited += 1;
        false
    }
}

/// Bytes actually occupied on disk.
#[must_use]
pub fn allocation(meta: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        meta.len()
    }
}

fn is_multilink(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.nlink() > 1
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

fn entry_name(entry: &fs::DirEntry) -> String {
    entry.file_name().to_string_lossy().into_owned()
}

fn root_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |s| s.to_string_lossy().into_owned(),
    )
}
