//! The walker.
//!
//! Physical-size accounting is the point of this module. A file's size on disk is
//! `st_blocks * 512`, not `st_size`: sparse files occupy less, and the last block
//! is rounded up. Hard links are counted once per inode; the extra names appear as
//! zero-size links so the tree still shows them.
//!
//! Directories are walked in parallel with rayon. Symlinks are never followed, so
//! a symlink loop cannot hang a scan and a link costs only its own metadata.
//!
//! # Why the tree is built twice
//!
//! The first version returned a `Vec<Node>` per directory and merged each child's
//! vector into its parent's, rewriting indices as it went. That copies every node
//! once per ancestor level, so a deep tree pays depth-many copies of its whole
//! node array. Measured: 51k files in 446 ms in one tree, but 972k files in 106 s
//! in another. The fix is to walk into a nested [`Scanned`] tree first, where a
//! child's descendants sit behind a pointer and are never re-copied, and then
//! flatten that tree into the arena exactly once.

use crate::model::{Node, NodeId, NodeKind, Tree};
use rayon::prelude::*;
#[cfg(unix)]
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(unix)]
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
    /// Optional wall-clock deadline for the whole walk. Past it, every directory still
    /// to enter is returned as a `Restricted` node with [`DEADLINE_NOTE`] — the same
    /// unmeasured-is-reported shape as a permission failure, never a silent zero.
    /// The caller cannot rely on killing the process (an elevated walker belongs to
    /// root), so the walker enforces its own budget, exactly like `max_entries`.
    pub deadline: Option<Instant>,
}

/// The `note` a deadline-hit directory carries. Public so the caller that set the
/// deadline can recognise its own budget in the result.
pub const DEADLINE_NOTE: &str = "deadline reached";

/// One finished child of the scan root, reported the moment it completes so a UI can
/// draw it without waiting for the whole tree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BranchDone {
    pub name: String,
    pub bytes: u64,
    pub files: u64,
}

pub type ProgressFn = Arc<dyn Fn(BranchDone) + Send + Sync>;

/// Scan `root` and return the whole tree. The root itself may be a file.
pub fn scan(root: &Path, opts: &ScanOptions) -> Result<Tree, ScanError> {
    scan_with_progress(root, opts, None)
}

/// As [`scan`], but reports every direct child of the root as soon as that child's
/// subtree has been measured.
pub fn scan_with_progress(
    root: &Path,
    opts: &ScanOptions,
    progress: Option<ProgressFn>,
) -> Result<Tree, ScanError> {
    let started = Instant::now();
    let meta = fs::symlink_metadata(root).map_err(|source| ScanError::Root {
        path: root.to_path_buf(),
        source,
    })?;

    let builder = Builder {
        // The inode ledger only exists where hard links are identified; on other platforms
        // there is nothing to dedupe and the field would be dead code.
        #[cfg(unix)]
        seen: Mutex::new(HashSet::new()),
        max_entries: opts.max_entries,
        deadline: opts.deadline,
        visited: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        progress,
    };

    let name = root_name(root);
    let scanned = if meta.is_dir() {
        builder.walk_dir(root, name, 0)
    } else {
        builder.leaf(name, &meta)
    };

    // One allocation for the arena: count first, then fill. Counting walks the
    // in-memory tree only, so it costs no syscalls.
    let mut nodes: Vec<Node> = Vec::with_capacity(count_nodes(&scanned));
    let root_id = flatten(scanned, &mut nodes, None);

    Ok(Tree {
        nodes,
        root: root_id,
        scanned_root: root.to_path_buf(),
        duration_ms: started.elapsed().as_millis() as u64,
        errors: builder.errors.load(Ordering::Relaxed),
    })
}

/// The nested form of the tree, used only during a scan.
struct Scanned {
    name: String,
    kind: NodeKind,
    own_bytes: u64,
    total_bytes: u64,
    file_count: u64,
    partial: bool,
    link_skipped: bool,
    missing: bool,
    newest_modified: Option<u64>,
    note: Option<String>,
    children: Vec<Scanned>,
}

fn count_nodes(node: &Scanned) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

/// Move the nested tree into the flat arena exactly once: pre-order, so a node's id
/// is always allocated before its children's.
fn flatten(scanned: Scanned, out: &mut Vec<Node>, parent: Option<NodeId>) -> NodeId {
    let id = out.len();
    let Scanned {
        name,
        kind,
        own_bytes,
        total_bytes,
        file_count,
        partial,
        link_skipped,
        missing,
        newest_modified,
        note,
        children,
    } = scanned;

    out.push(Node {
        name,
        kind,
        own_bytes,
        total_bytes,
        file_count,
        children: Vec::with_capacity(children.len()),
        parent,
        partial,
        link_skipped,
        missing,
        newest_modified,
        note,
    });

    for child in children {
        let child_id = flatten(child, out, Some(id));
        out[id].children.push(child_id);
    }
    id
}

struct Builder {
    #[cfg(unix)]
    seen: Mutex<HashSet<(u64, u64)>>,
    max_entries: Option<u64>,
    deadline: Option<Instant>,
    visited: AtomicU64,
    errors: AtomicU64,
    progress: Option<ProgressFn>,
}

impl Builder {
    /// Report a completed branch, but only for the root's own children.
    fn report(&self, scanned: &Scanned) {
        if let Some(progress) = &self.progress {
            progress(BranchDone {
                name: scanned.name.clone(),
                bytes: scanned.total_bytes,
                files: scanned.file_count,
            });
        }
    }

    fn walk_dir(&self, path: &Path, name: String, depth: usize) -> Scanned {
        // The budget is checked before the directory is opened, so a spent deadline
        // costs no syscalls and produces the same Restricted shape as a refusal.
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.errors.fetch_add(1, Ordering::Relaxed);
            return restricted(name, path, DEADLINE_NOTE.to_string());
        }

        let mut children: Vec<Scanned> = Vec::new();
        let mut own_bytes = 0u64;
        let mut partial = false;

        let read_dir = match fs::read_dir(path) {
            Ok(rd) => rd,
            Err(err) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                return restricted(name, path, err.to_string());
            }
        };

        let mut dirs: Vec<fs::DirEntry> = Vec::new();
        let mut files: Vec<fs::DirEntry> = Vec::new();

        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    partial = true;
                    continue;
                }
            };
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => dirs.push(entry),
                Ok(_) => files.push(entry),
                Err(_) => {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    partial = true;
                }
            }
        }

        // Deterministic order, so two scans of an unchanged tree agree.
        //
        // `sort_by_cached_key` rather than `sort_by_key`: the key is an owned OsString,
        // and `sort_by_key` recomputes it on every comparison, so a directory with half a
        // million entries allocates tens of millions of strings just to sort itself.
        dirs.sort_by_cached_key(fs::DirEntry::file_name);
        files.sort_by_cached_key(fs::DirEntry::file_name);

        // The callback fires inside the closure, so a branch is reported the instant it
        // finishes rather than when `collect` finally returns.
        let subtrees: Vec<Scanned> = dirs
            .par_iter()
            .map(|entry| self.walk_dir(&entry.path(), entry_name(entry), depth + 1))
            .collect();

        for sub in subtrees {
            if sub.partial {
                partial = true;
            }
            children.push(sub);
        }

        for entry in &files {
            // `DirEntry::metadata` does not traverse symlinks, which is exactly what we
            // want: a link costs only its own metadata, never its target.
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    partial = true;
                    continue;
                }
            };
            if !self.claim_entry() {
                partial = true;
                break;
            }
            let leaf = self.leaf(entry_name(entry), &meta);
            if depth == 0 {
                self.report(&leaf);
            }
            children.push(leaf);
        }

        // The directory's own metadata blocks are real occupied space.
        if let Ok(meta) = fs::metadata(path) {
            own_bytes += allocation(&meta);
        }

        let own_modified = fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let newest_modified = children
            .iter()
            .filter_map(|c| c.newest_modified)
            .max()
            .into_iter()
            .chain(own_modified)
            .max();

        let child_total: u64 = children.iter().map(|c| c.total_bytes).sum();
        // Leaves count 1 each, so summing the children's counts is the whole subtree.
        let file_count: u64 = children.iter().map(|c| c.file_count).sum();

        let scanned = Scanned {
            name,
            kind: NodeKind::Directory,
            own_bytes,
            total_bytes: own_bytes.saturating_add(child_total),
            file_count,
            partial,
            link_skipped: false,
            missing: false,
            newest_modified,
            note: None,
            children,
        };

        // Depth 1 is a direct child of the scan root: exactly the branches the first
        // view draws.
        if depth == 1 {
            self.report(&scanned);
        }

        scanned
    }

    fn leaf(&self, name: String, meta: &fs::Metadata) -> Scanned {
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

        Scanned {
            name,
            kind: if is_link {
                NodeKind::Symlink
            } else {
                NodeKind::File
            },
            own_bytes,
            total_bytes: own_bytes,
            file_count: 1,
            partial: false,
            link_skipped: duplicated,
            missing: false,
            newest_modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            note,
            children: Vec::new(),
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

    /// False once the optional entry cap is reached.
    fn claim_entry(&self) -> bool {
        let Some(limit) = self.max_entries else {
            return true;
        };
        self.visited.fetch_add(1, Ordering::Relaxed) < limit
    }
}

/// A directory that could not be entered: still occupies blocks of its own, still
/// reported, still flags every ancestor partial. Produced by permission failures and
/// by an expired deadline alike.
fn restricted(name: String, path: &Path, note: String) -> Scanned {
    // A directory we cannot enter still occupies blocks of its own.
    let own_bytes = fs::symlink_metadata(path).map_or(0, |m| allocation(&m));
    Scanned {
        name,
        kind: NodeKind::Restricted,
        own_bytes,
        total_bytes: own_bytes,
        file_count: 0,
        partial: true,
        link_skipped: false,
        missing: false,
        newest_modified: None,
        note: Some(note),
        children: Vec::new(),
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
