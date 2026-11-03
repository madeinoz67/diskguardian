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
use std::sync::Mutex;
use std::time::Instant;

/// How many times a syscall interrupted by a signal is retried before the directory is
/// treated as unreadable. A bound rather than an infinity: a tree that answers `EINTR`
/// forever is unreadable in practice, and a scan must not spin on it.
const INTERRUPT_RETRIES: u32 = 8;

/// Run a directory syscall, retrying the one error that is not about the directory.
///
/// `EINTR` means a signal arrived mid-call; it says nothing about permission or existence.
/// Under the parallel walk it is routine, and treating it as a permission wall lies in the
/// worst direction — it reports measurable space as unmeasurable, draws a purple petal on a
/// directory the user can open, and can offer an administrator pass for nothing.
///
/// Measured 2026-11-02 on a ~1M-node tree: a ten-thread scan produced 10–30 of these per
/// pass, hiding 0–1.8 GB while a serial walk and `du` saw none. One thread produced one.
fn retry_interrupted<T>(mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut interruptions = 0u32;
    loop {
        match op() {
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                interruptions += 1;
                if interruptions > INTERRUPT_RETRIES {
                    return Err(err);
                }
            }
            other => return other,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot read {path}: {source}")]
    Root {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The caller asked for the walk to stop. Deliberately not a partial tree: see
    /// [`ScanOptions::cancel`].
    #[error("scan cancelled")]
    Cancelled,
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
    /// Set by the caller to abandon the walk — the user picked another directory. Unlike a
    /// deadline, cancellation is not a partial answer: if the walk observes the request, it
    /// returns [`ScanError::Cancelled`] and no tree at all, because a half-built tree shown as
    /// a scan would be a claim about the disk that nobody measured.
    ///
    /// A request that arrives after the last poll yields a completed tree — `Ok` therefore
    /// means "not truncated for cancellation", never "the request was received". Both
    /// outcomes are honest; which one a given race produces is not stable across runs.
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Volume-scan boundary (C29): the root's device id defines the scan. A directory on
    /// another device is another volume's content, not unmeasurable space of this one, so
    /// it is not entered and not added to the tree — it is listed in
    /// [`Tree::skipped_mounts`] instead, because excluded is a third state, distinct from
    /// both measured and restricted.
    ///
    /// Unix-only in effect: std exposes `st_dev` on Unix. On Windows a drive root is a
    /// mount point with nothing nested, so the bound is vacuous there by construction —
    /// stated rather than pretended.
    pub same_device_only: bool,
    /// Subtrees to skip by path prefix (component-wise), with the same listed-not-dropped
    /// treatment. The app supplies the two fixed macOS prefixes that share the boot
    /// group's device id — the Data volume's own mount, and the transient local-snapshot
    /// mounts — which the device bound alone cannot stop.
    pub exclude_prefixes: Vec<PathBuf>,
}

/// The `note` a deadline-hit directory carries. Public so the caller that set the
/// deadline can recognise its own budget in the result.
pub const DEADLINE_NOTE: &str = "deadline reached";

/// The `note` a directory carries when the walk was abandoned under it. Only ever seen inside
/// a tree that is then discarded, but a note is cheaper than a silent empty directory if that
/// ever stops being true.
pub const CANCELLED_NOTE: &str = "scan cancelled";

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
    // Follow the ROOT if it is a symlink, and only the root.
    //
    // `/tmp` is a symlink to `/private/tmp` on macOS, so `symlink_metadata` reported the
    // root as a link, the walker treated it as a single file, and scanning `/tmp` returned
    // zero files while Finder showed plenty. Inside the walk symlinks are still never
    // followed: a link costs its own metadata and nothing more.
    let meta = fs::metadata(root)
        .or_else(|_| fs::symlink_metadata(root))
        .map_err(|source| ScanError::Root {
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
        cancel: opts.cancel.clone(),
        cancelled: std::sync::atomic::AtomicBool::new(false),
        visited: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        progress,
        // The device bound is captured from the root's own metadata — the same `meta`
        // the walk already took — so "what is this scan" is fixed before anything is
        // walked. Only read when `same_device_only` asked for a bounded walk.
        root_dev: device_id(&meta),
        exclude_prefixes: if opts.same_device_only || !opts.exclude_prefixes.is_empty() {
            opts.exclude_prefixes.clone()
        } else {
            Vec::new()
        },
        skipped: Mutex::new(Vec::new()),
    };

    // Checked before the walk as well as inside it, because a file root has no directory to
    // poll: dropping a single file on the window with a request already standing used to
    // return a completed scan. Both root shapes answer the same way.
    if builder.cancel_requested() {
        return Err(ScanError::Cancelled);
    }

    let name = root_name(root);
    let scanned = if meta.is_dir() {
        builder.walk_dir(root, name, 0)
    } else {
        builder.leaf(name, &meta)
    };

    // An abandoned walk produces no tree. Returning what it had reached would be a partial
    // measurement presented as a scan of the volume. The invariant that makes the remaining
    // race benign: every cancellation-truncation site sets `cancelled`, so an `Ok` tree was
    // not truncated for cancellation — it is as complete as an uncancelled walk.
    if builder.cancelled.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }

    // One allocation for the arena: count first, then fill. Counting walks the
    // in-memory tree only, so it costs no syscalls.
    let mut nodes: Vec<Node> = Vec::with_capacity(count_nodes(&scanned));
    let root_id = flatten(scanned, &mut nodes, None);

    // The boundary's exclusions, in walk order for determinism (the walk is parallel,
    // so the raw order is not), and deduplicated because a path is visited once.
    let mut skipped_mounts = builder
        .skipped
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    skipped_mounts.sort();
    skipped_mounts.dedup();

    Ok(Tree {
        nodes,
        root: root_id,
        scanned_root: root.to_path_buf(),
        duration_ms: started.elapsed().as_millis() as u64,
        errors: builder.errors.load(Ordering::Relaxed),
        skipped_mounts,
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
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Set once the walk has actually seen a cancellation request, so the caller can tell a
    /// completed walk from an abandoned one without reading a flag it might have raced.
    cancelled: std::sync::atomic::AtomicBool,
    visited: AtomicU64,
    errors: AtomicU64,
    progress: Option<ProgressFn>,
    /// The scan root's device id when a volume scan asked to be bounded by it. `None`
    /// off Unix (std exposes `st_dev` on Unix only) — there the bound is vacuous, which
    /// is also the truth: a Windows drive root is a mount point with nothing nested.
    root_dev: Option<u64>,
    exclude_prefixes: Vec<PathBuf>,
    /// The mounts and prefixes the boundary excluded, in walk order; sorted and deduped
    /// once at the end. `Mutex` because the walk is parallel.
    skipped: Mutex<Vec<String>>,
}

/// The device a piece of metadata lives on, where the platform tells us. Unix only:
/// `st_dev` has no std equivalent on Windows, so the device bound degrades to a no-op
/// there rather than to a guess.
fn device_id(meta: &fs::Metadata) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(meta.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

impl Builder {
    /// Report a completed branch, but only for the root's own children.
    ///
    /// Silent once the walk has been abandoned: a cancelled walk's branches are not news, and
    /// on a shared progress channel they are indistinguishable from the next scan's.
    fn report(&self, scanned: &Scanned) {
        if self.cancelled.load(Ordering::Relaxed) || self.cancel_requested() {
            return;
        }
        if let Some(progress) = &self.progress {
            progress(BranchDone {
                name: scanned.name.clone(),
                bytes: scanned.total_bytes,
                files: scanned.file_count,
            });
        }
    }

    /// Whether the caller has asked the walk to stop. Cheap enough to ask per directory:
    /// one relaxed load, no syscall.
    fn cancel_requested(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
    }

    /// Whether the volume boundary stops the walk at this directory. Prefixes match by
    /// path components, never by characters: excluding `/x/skip` must not swallow
    /// `/x/skipme` beside it.
    fn boundary_excluded(&self, entry: &fs::DirEntry) -> bool {
        let path = entry.path();
        if self
            .exclude_prefixes
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            self.record_skipped(&path);
            return true;
        }
        if self.crosses_device(entry) {
            self.record_skipped(&path);
            return true;
        }
        false
    }

    /// Whether this entry lives on a device the scan was not asked to measure. An entry
    /// whose metadata cannot be read is *not* excluded on that account — the walk
    /// proceeds and reports it restricted if it truly cannot be entered, which is A3's
    /// shape, not this bound's.
    fn crosses_device(&self, entry: &fs::DirEntry) -> bool {
        let Some(root_dev) = self.root_dev else {
            return false;
        };
        match entry.metadata().ok().and_then(|meta| device_id(&meta)) {
            Some(dev) => dev != root_dev,
            None => false,
        }
    }

    /// Record a boundary exclusion. Reported, never silently dropped (A3): the tree's
    /// `skipped_mounts` is the panel's list.
    fn record_skipped(&self, path: &Path) {
        self.skipped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(path.display().to_string());
    }

    fn walk_dir(&self, path: &Path, name: String, depth: usize) -> Scanned {
        // Cancellation unwinds from here rather than measuring anything: every level still
        // in flight returns immediately, so the whole walk stops in one pass down the tree.
        if self.cancel_requested() {
            self.cancelled.store(true, Ordering::Relaxed);
            return restricted(name, path, CANCELLED_NOTE.to_string());
        }

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

        // An interrupted `read_dir` is retried; anything else is a real answer about the
        // directory and becomes a restricted node.
        let read_dir = match retry_interrupted(|| fs::read_dir(path)) {
            Ok(rd) => rd,
            Err(err) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                return restricted(name, path, err.to_string());
            }
        };

        let mut dirs: Vec<fs::DirEntry> = Vec::new();
        let mut files: Vec<fs::DirEntry> = Vec::new();

        let mut entries = read_dir;
        let mut interruptions = 0u32;
        loop {
            // Also checked per entry: one huge flat directory would otherwise hold the walk
            // to the end of its listing before the next directory boundary noticed.
            if self.cancel_requested() {
                self.cancelled.store(true, Ordering::Relaxed);
                // The truncated listing is not a complete one. Today every cancelled walk
                // ends in `Err` so this flag is unused, but a tree-on-cancel would be a
                // C5/A3 violation without it, and the sibling truncations all set it.
                partial = true;
                break;
            }
            let entry = match entries.next() {
                None => break,
                Some(Ok(entry)) => entry,
                // The same rule as the open: a signal is not a defect in the tree, so ask
                // again. Bounded, so a directory that always answers `EINTR` is counted as
                // a failure rather than hanging the scan.
                Some(Err(err))
                    if err.kind() == std::io::ErrorKind::Interrupted
                        && interruptions < INTERRUPT_RETRIES =>
                {
                    interruptions += 1;
                    continue;
                }
                Some(Err(_)) => {
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

        // The volume boundary (C29), applied before anything is dispatched: a directory
        // on another device, or under an excluded prefix, is another volume's content,
        // not unmeasurable space of this one. It is not walked and not added to the
        // tree; it is recorded, because excluded is a third state — distinct from both
        // measured and restricted — and the panel names it. Neither check costs a
        // syscall the walk was not already making: the metadata this reads is the same
        // one `walk_dir` re-reads for `own_bytes`.
        let mut bounded_dirs: Vec<fs::DirEntry> = Vec::with_capacity(dirs.len());
        for entry in dirs {
            if self.boundary_excluded(&entry) {
                continue;
            }
            bounded_dirs.push(entry);
        }
        let dirs = bounded_dirs;

        // Checked before the dispatch as well as inside it: spawning a task per queued
        // directory after the user has abandoned the walk is work nobody asked for, and on a
        // wide directory it is thousands of tasks.
        let subtrees: Vec<Scanned> = if self.cancel_requested() {
            self.cancelled.store(true, Ordering::Relaxed);
            Vec::new()
        } else {
            // The callback fires inside the closure, so a branch is reported the instant it
            // finishes rather than when `collect` finally returns.
            dirs.par_iter()
                .map(|entry| self.walk_dir(&entry.path(), entry_name(entry), depth + 1))
                .collect()
        };

        for sub in subtrees {
            if sub.partial {
                partial = true;
            }
            children.push(sub);
        }

        for entry in &files {
            // The measurement loop is where a cancelled walk used to keep working: the
            // listing could be abandoned and this loop would still stat every file already
            // collected — measured at 3.9 µs per entry on warm APFS, so seconds on a wide
            // directory and far longer on cold media.
            if self.cancel_requested() {
                self.cancelled.store(true, Ordering::Relaxed);
                partial = true;
                break;
            }
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

/// Shared with the duplicate finder: a file with more than one name is one inode,
/// and deleting any single name frees nothing.
pub(crate) fn is_multilink(meta: &fs::Metadata) -> bool {
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

#[cfg(test)]
mod interrupt_tests {
    use super::{retry_interrupted, INTERRUPT_RETRIES};
    use std::io;

    fn interrupted<T>() -> io::Result<T> {
        Err(io::Error::from(io::ErrorKind::Interrupted))
    }

    #[test]
    fn an_interrupted_call_is_retried_until_it_answers() {
        let mut calls = 0;
        let result = retry_interrupted(|| {
            calls += 1;
            if calls < 3 {
                interrupted()
            } else {
                Ok(7)
            }
        });
        assert_eq!(result.ok(), Some(7), "a signal is not an answer");
        assert_eq!(calls, 3, "the call is asked again, not abandoned");
    }

    #[test]
    fn a_persistent_interruption_gives_up_rather_than_spinning() {
        let mut calls = 0;
        let result: io::Result<()> = retry_interrupted(|| {
            calls += 1;
            interrupted()
        });
        assert!(result.is_err(), "the error survives the retries");
        assert_eq!(
            calls,
            INTERRUPT_RETRIES + 1,
            "bounded: a directory that always answers EINTR must not hang the scan"
        );
    }

    #[test]
    fn a_real_error_is_answered_the_first_time() {
        // The property that keeps the honesty invariant pointing the right way: a genuine
        // permission wall must become a restricted node immediately, never be retried into
        // a timeout, and never be swallowed.
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::NotFound,
            io::ErrorKind::NotADirectory,
        ] {
            let mut calls = 0;
            let result: io::Result<()> = retry_interrupted(|| {
                calls += 1;
                Err(io::Error::from(kind))
            });
            assert_eq!(result.err().map(|e| e.kind()), Some(kind));
            assert_eq!(
                calls, 1,
                "{kind:?} is a statement about the tree, not a signal"
            );
        }
    }
}
