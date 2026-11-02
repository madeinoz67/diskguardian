//! Duplicate files: a cheap-first pass over a finished scan.
//!
//! The pipeline runs on demand, never during a scan, and every step costs less than
//! the one after it:
//!
//! 1. **Group by allocated size.** Free: `total_bytes` came out of the scan. Only
//!    files whose *allocation* is equal can be duplicates, so every size singleton
//!    is settled before a single byte is read. Allocation is coarser than length
//!    (two files of different lengths round to the same block count), so a size
//!    group is a *candidate* set, not a match.
//! 2. **Head and tail hash** — the first and last 8 KiB of each candidate, keyed
//!    with the file's logical length. At most 16 KiB of I/O per file, and it
//!    eliminates nearly everything without reading whole files.
//! 3. **Full hash** — BLAKE3 over the whole file, only for candidates whose length,
//!    head and tail all agree. Two files can agree at both ends and differ in the
//!    middle (the uncovered band of a 20 KiB file is 4 KiB), so this stage is what
//!    makes the final claim safe.
//!
//! # What a "duplicate" honestly means here
//!
//! A reported group is **content-identical**: the whole-file BLAKE3 digests match.
//! It is *not* a promise of reclaimable space:
//!
//! * APFS clones copy-on-write files that hash identical but occupy shared blocks;
//!   deleting one member frees nothing. Clone detection needs `ATTR_CMNEXT_CLONEPARENT`,
//!   which this version does not have, so [`CONTENT_IDENTICAL_CAVEAT`] rides on every
//!   group and no consumer can show a group without it.
//! * Hard links are not duplicates: the scanner already marks extra names
//!   `link_skipped` at zero bytes, and the name that carries the bytes is itself
//!   excluded here by a fresh `nlink` check at candidate time, because deleting one
//!   name of a shared inode frees nothing.
//!
//! The pass is read-only. It proposes nothing; deletion stays behind the collector
//! and the guard.

use crate::model::{NodeId, NodeKind, Tree};
use crate::scan::is_multilink;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// How much of each end of a file the cheap stage reads: 8 KiB at the head and,
/// for files longer than that, 8 KiB at the tail — at most 16 KiB before a full
/// hash is ever attempted.
const EDGE_BYTES: u64 = 8 * 1024;

/// The caveat every group carries. A group is a statement about content, and APFS
/// clones mean content-identical files can share physical storage, so deleting a
/// member can free less than the group's `wasted_bytes`. Public so the UI shows the
/// same words the engine believes.
pub const CONTENT_IDENTICAL_CAVEAT: &str =
    "content-identical (whole-file BLAKE3); some members may share physical storage \
     (APFS clones), so deleting a member can free less than the wasted figure";

/// One step of the pass, reported the moment it happens so a UI can show progress.
///
/// The callback fires for every file whose bytes are about to be read, and for no
/// others, which makes it also the seam that proves which files a pass opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateStage {
    /// The 8 KiB head/tail read of a candidate.
    HeadTail,
    /// The whole-file BLAKE3 of a candidate whose edges matched.
    Full,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateProgress {
    pub path: PathBuf,
    pub stage: DuplicateStage,
}

/// Mirrors the scanner's [`crate::scan::ProgressFn`] shape: a shared callback the
/// caller supplies, called from the pass as work happens.
pub type DuplicateProgressFn = Arc<dyn Fn(DuplicateProgress) + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub struct DuplicateOptions {
    /// Candidates below this *allocated* size are never hashed (physical bytes, not
    /// length — the same measure the tree shows). Keeps the pass off small files,
    /// which are numerous and worth little.
    pub min_bytes: u64,
    /// Cap on the number of candidate files whose bytes are read. `None` means no
    /// cap. Reaching the cap stops the pass and marks the report partial, because
    /// more duplicates may exist unexamined.
    pub max_candidates: Option<usize>,
    /// Optional wall-clock deadline, checked between files. Past it the pass stops
    /// and the report is marked partial. A single whole-file hash already underway
    /// is not interruptible; the budget is enforced per file.
    pub deadline: Option<Instant>,
}

impl DuplicateOptions {
    /// Default budget for an interactive "find duplicates" click: small files are
    /// skipped, work is bounded only by the tree the user already scanned.
    #[must_use]
    pub fn new(min_bytes: u64) -> Self {
        Self {
            min_bytes,
            max_candidates: None,
            deadline: None,
        }
    }
}

impl Default for DuplicateOptions {
    fn default() -> Self {
        // 1 MiB, not 0: a default that hashed every size group would make the first
        // click on a big tree read for minutes when the user asked a question about
        // space, most of which lives in big files anyway.
        Self::new(1024 * 1024)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateMember {
    pub node: NodeId,
    pub path: PathBuf,
    /// True for the one member this report suggests keeping: the newest file in the
    /// group. A copy nobody has touched recently is the likelier duplicate; the file
    /// someone still writes to is the one they still use. Ties and unknown mtimes
    /// fall to the lowest node id, so the choice is deterministic.
    pub keep: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    /// Allocated bytes of one member — the tree's figure, the same number the
    /// flower shows for it.
    pub bytes_each: u64,
    /// `bytes_each * (members - 1)`: what deleting every member but the keeper
    /// would free *if* no two members share storage. The caveat below is why that
    /// "if" is written out.
    pub wasted_bytes: u64,
    pub members: Vec<DuplicateMember>,
    /// Always [`CONTENT_IDENTICAL_CAVEAT`]. Carried per group so no consumer can
    /// show a group without the honesty attached to it.
    pub caveat: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateReport {
    /// Groups, most wasted bytes first.
    pub groups: Vec<DuplicateGroup>,
    /// True when the pass stopped early (deadline or candidate cap). More
    /// duplicates may exist; every group present was fully verified.
    pub partial: bool,
    /// Candidate files whose bytes were read (a file counts once, however many
    /// stages read it).
    pub files_hashed: u64,
    /// Candidates that could not be read, or stopped being files since the scan.
    pub errors: u64,
    /// Candidates dropped because they are one name of a shared-inode hard link.
    pub excluded_links: u64,
    pub duration_ms: u64,
}

/// The scan root itself could not be read: the volume or directory the tree was
/// scanned from is gone, so every candidate read would fail for the same reason.
/// Per-file failures are not this — they are counted in [`DuplicateReport::errors`].
#[derive(Debug, thiserror::Error)]
#[error("cannot read {path}: {source}")]
pub struct DuplicateError {
    pub path: PathBuf,
    #[source]
    pub source: io::Error,
}

/// Find content-identical files in a finished scan. Read-only: this proposes
/// nothing, it reports; deletion stays behind the collector and the guard.
///
/// Cheap-first (see the module docs): the tree's own sizes settle everything they
/// can before a single byte is read, then 8 KiB edges, then whole files. A pass
/// that finds nothing still proves "no size twins above the floor", which costs no
/// I/O at all.
pub fn find_duplicates(
    tree: &Tree,
    opts: &DuplicateOptions,
) -> Result<DuplicateReport, DuplicateError> {
    find_duplicates_with_progress(tree, opts, None)
}

/// As [`find_duplicates`], reporting every file read the moment it happens. The
/// callback fires immediately before each read, and for no others — which is what
/// makes it the seam the tests assert opened files through.
pub fn find_duplicates_with_progress(
    tree: &Tree,
    opts: &DuplicateOptions,
    progress: Option<DuplicateProgressFn>,
) -> Result<DuplicateReport, DuplicateError> {
    let started = Instant::now();

    // Step 1: group by allocated size — the tree's own number, so this settles
    // every size singleton before a single byte is read. Files that are missing,
    // one of several names for an inode, or below the floor never become candidates.
    let mut by_size: HashMap<u64, Vec<NodeId>> = HashMap::new();
    for (i, node) in tree.nodes.iter().enumerate() {
        if node.kind != NodeKind::File || node.missing || node.link_skipped {
            continue;
        }
        if node.total_bytes < opts.min_bytes {
            continue;
        }
        by_size.entry(node.total_bytes).or_default().push(i);
    }

    let mut size_groups: Vec<(u64, Vec<NodeId>)> = by_size
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .collect();
    // Largest first, so a deadline or cap bites the small groups before the big ones.
    size_groups.sort_by(|a, b| b.0.cmp(&a.0).then(a.1[0].cmp(&b.1[0])));

    // Candidates exist and the scan root is gone: every read below would fail with
    // the same cause, so name it once instead of counting N unreadable files and
    // quietly reporting "no duplicates".
    if !size_groups.is_empty() {
        if let Err(source) = fs::symlink_metadata(&tree.scanned_root) {
            return Err(DuplicateError {
                path: tree.scanned_root.clone(),
                source,
            });
        }
    }

    let mut pass = Pass::new(opts, progress);
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    let mut partial = false;

    'all_groups: for (bytes_each, members) in size_groups {
        // Step 2: stat each candidate, drop what is no longer a plain single-name
        // file, hash the edges.
        let mut by_edges: HashMap<EdgeKey, Vec<(NodeId, PathBuf)>> = HashMap::new();
        // Set when the cap stops this group admitting more candidates: what has
        // already been read is still verified below (those files are inside the
        // cap), but no further group is started.
        let mut capped = false;
        for id in members {
            if !pass.edges_budget_left() {
                partial = true;
                capped = true;
                break;
            }
            let path = tree.path_of(id);
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => {
                    pass.errors += 1;
                    continue;
                }
            };
            // Not the file the scan saw (replaced by a symlink or a directory, or
            // vanished): the node's identity no longer holds, so it cannot honestly
            // join a group. Counted, not hidden.
            if !meta.is_file() {
                pass.errors += 1;
                continue;
            }
            // One inode, several names: deleting this name frees nothing, so it is
            // never proposed. The scanner already zeroed the extra names at walk
            // time; this check catches the name that carries the bytes.
            if is_multilink(&meta) {
                pass.excluded_links += 1;
                continue;
            }
            pass.begin_hash(&path, DuplicateStage::HeadTail);
            match hash_edges(&path, meta.len()) {
                Ok(key) => by_edges.entry(key).or_default().push((id, path)),
                Err(_) => pass.errors += 1,
            }
        }

        // Step 3: whole-file BLAKE3, only for candidates whose length and edges
        // already agree. This is the stage that catches files differing only in the
        // band between the head and tail reads.
        for (_key, matched) in by_edges {
            if matched.len() < 2 {
                continue;
            }
            let mut by_digest: HashMap<blake3::Hash, Vec<(NodeId, PathBuf)>> = HashMap::new();
            for (id, path) in matched {
                if !pass.full_budget_left() {
                    partial = true;
                    break 'all_groups;
                }
                pass.report(&path, DuplicateStage::Full);
                match hash_file(&path) {
                    Ok(digest) => by_digest.entry(digest).or_default().push((id, path)),
                    Err(_) => pass.errors += 1,
                }
            }
            for (_digest, files) in by_digest {
                if files.len() < 2 {
                    continue;
                }
                groups.push(build_group(bytes_each, files, tree));
            }
        }

        if capped {
            break 'all_groups;
        }
    }

    groups.sort_by(|a, b| {
        b.wasted_bytes
            .cmp(&a.wasted_bytes)
            .then(b.bytes_each.cmp(&a.bytes_each))
            .then(a.members[0].node.cmp(&b.members[0].node))
    });

    Ok(DuplicateReport {
        groups,
        partial,
        files_hashed: pass.hashed,
        errors: pass.errors,
        excluded_links: pass.excluded_links,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// The running budget and counters of one pass.
struct Pass {
    deadline: Option<Instant>,
    max_candidates: Option<usize>,
    hashed: u64,
    errors: u64,
    excluded_links: u64,
    progress: Option<DuplicateProgressFn>,
}

impl Pass {
    fn new(opts: &DuplicateOptions, progress: Option<DuplicateProgressFn>) -> Self {
        Self {
            deadline: opts.deadline,
            max_candidates: opts.max_candidates,
            hashed: 0,
            errors: 0,
            excluded_links: 0,
            progress,
        }
    }

    fn deadline_left(&self) -> bool {
        self.deadline
            .is_none_or(|deadline| Instant::now() < deadline)
    }

    /// Whether a *new* candidate may be read: the deadline holds and the cap has
    /// room. Checked before the file is opened.
    fn edges_budget_left(&self) -> bool {
        self.deadline_left()
            && self
                .max_candidates
                .is_none_or(|cap| (self.hashed as usize) < cap)
    }

    /// Whether an already-counted candidate may proceed to its whole-file hash. The
    /// deadline still applies; the cap does not, because this file was counted when
    /// its edges were read and the full hash is not a new candidate.
    fn full_budget_left(&self) -> bool {
        self.deadline_left()
    }

    /// Count a file as hashed and report it, immediately before its first read.
    fn begin_hash(&mut self, path: &Path, stage: DuplicateStage) {
        self.hashed += 1;
        self.report(path, stage);
    }

    fn report(&self, path: &Path, stage: DuplicateStage) {
        if let Some(progress) = &self.progress {
            progress(DuplicateProgress {
                path: path.to_path_buf(),
                stage,
            });
        }
    }
}

/// Length plus the digests of both edge reads. The length is part of the key
/// because allocation groups are coarser than lengths: two files can allocate the
/// same blocks and still hold different numbers of bytes.
type EdgeKey = (u64, blake3::Hash, blake3::Hash);

/// Hash the first `EDGE_BYTES` of the file and, when it is longer than that, the
/// last `EDGE_BYTES` — at most 16 KiB of I/O.
fn hash_edges(path: &Path, len: u64) -> io::Result<EdgeKey> {
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; EDGE_BYTES as usize];

    let head_len = read_at_most(&mut file, &mut buf)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&buf[..head_len]);
    let head = hasher.finalize();

    let tail = if len > EDGE_BYTES {
        file.seek(SeekFrom::Start(len - EDGE_BYTES))?;
        let tail_len = read_at_most(&mut file, &mut buf)?;
        hasher.update(&buf[..tail_len]);
        hasher.finalize()
    } else {
        // The head read already covered the whole file.
        head
    };

    Ok((len, head, tail))
}

/// BLAKE3 over the whole file, streamed through a 64 KiB buffer.
fn hash_file(path: &Path) -> io::Result<blake3::Hash> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = read_at_most(&mut file, &mut buf)?;
        if n == 0 {
            return Ok(hasher.finalize());
        }
        hasher.update(&buf[..n]);
    }
}

/// Read until the buffer is full or the file ends; short kernel reads are normal.
fn read_at_most(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

fn build_group(bytes_each: u64, files: Vec<(NodeId, PathBuf)>, tree: &Tree) -> DuplicateGroup {
    let mut members: Vec<DuplicateMember> = files
        .into_iter()
        .map(|(node, path)| DuplicateMember {
            node,
            path,
            keep: false,
        })
        .collect();
    members.sort_by_key(|m| m.node);

    // The keeper is the newest file; see `DuplicateMember::keep`. Unknown mtimes
    // count as the oldest, because unreadable age must not win the choice. Ties
    // fall to the lowest node id, and the walker's order (parents before children,
    // entries sorted by name) makes that deterministic for a given tree.
    let keeper = members
        .iter()
        .enumerate()
        .max_by(|(index_a, a), (index_b, b)| {
            let age_a = tree.node(a.node).newest_modified.unwrap_or(u64::MIN);
            let age_b = tree.node(b.node).newest_modified.unwrap_or(u64::MIN);
            age_a.cmp(&age_b).then(index_b.cmp(index_a))
        })
        .expect("a group is only built from two or more files")
        .0;
    members[keeper].keep = true;

    let wasted_bytes = bytes_each.saturating_mul(members.len() as u64 - 1);
    DuplicateGroup {
        bytes_each,
        wasted_bytes,
        members,
        caveat: CONTENT_IDENTICAL_CAVEAT,
    }
}
