//! Cleanup candidates and queries over a finished scan.
//!
//! Two separate ideas, deliberately kept apart:
//!
//! * [`candidates`] answers "what is likely regenerable or disposable", from names and
//!   context only. It never assumes that *old* means disposable, because the user may well
//!   want to remove something they created this morning.
//! * [`find`] answers "show me things matching a filter" — size, age, name — so age is
//!   something the caller chooses, in either direction, rather than a policy baked in here.
//!
//! Both are read-only and both return paths that a human still has to act on.

use crate::model::{NodeId, Tree};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// Build output that a toolchain can recreate.
    Regenerable,
    /// A cache directory.
    Cache,
    /// A backup or archive of something else.
    Backup,
    /// A disk image.
    Image,
    /// The system trash.
    Trash,
}

impl CandidateKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Regenerable => "regenerable",
            Self::Cache => "cache",
            Self::Backup => "backup",
            Self::Image => "disk image",
            Self::Trash => "trash",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Match {
    pub node: NodeId,
    pub path: PathBuf,
    pub bytes: u64,
    pub newest_modified: Option<u64>,
    /// Present for rule-based hits, absent for plain filter matches.
    pub kind: Option<CandidateKind>,
    pub reason: String,
}

/// Which side of an age bound to look on. Either direction, because "recent but unwanted"
/// is as real a question as "old and forgotten".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeFilter {
    OlderThanDays(u64),
    NewerThanDays(u64),
}

impl AgeFilter {
    fn day_seconds() -> u64 {
        86_400
    }

    fn matches(self, newest_modified: Option<u64>, now: u64) -> bool {
        let Some(modified) = newest_modified else {
            // Unreadable age never satisfies a filter the user asked for; saying yes would
            // mean proposing deletions on evidence we do not have.
            return false;
        };
        let days = self.days();
        let cutoff = now.saturating_sub(days.saturating_mul(Self::day_seconds()));
        match self {
            Self::OlderThanDays(_) => modified < cutoff,
            Self::NewerThanDays(_) => modified >= cutoff,
        }
    }

    fn days(self) -> u64 {
        match self {
            Self::OlderThanDays(d) | Self::NewerThanDays(d) => d,
        }
    }

    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::OlderThanDays(d) => format!("untouched for {d} days"),
            Self::NewerThanDays(d) => format!("changed in the last {d} days"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClassifyOptions {
    /// Anything smaller is not worth proposing; keeps the list readable.
    pub min_bytes: u64,
    /// `None` means any age. The caller decides whether age matters.
    pub age: Option<AgeFilter>,
    /// Passed in rather than read from the clock, so tests are deterministic.
    pub now: u64,
}

impl Default for ClassifyOptions {
    fn default() -> Self {
        Self {
            min_bytes: 100 * 1024 * 1024,
            age: None,
            now: 0,
        }
    }
}

/// Names that almost always mean "a toolchain can rebuild this".
const REGENERABLE: &[&str] = &[
    "deriveddata",
    "node_modules",
    "target",
    "build",
    ".gradle",
    ".next",
    "__pycache__",
    ".tox",
    ".pytest_cache",
    "bazel-out",
    ".turbo",
    "dist",
];

const CACHES: &[&str] = &["caches", "cache", ".cache"];

const IMAGE_EXTENSIONS: &[&str] = &["dmg", "vmdk", "qcow2", "vdi", "iso", "sparseimage"];

/// Rule-based cleanup candidates.
///
/// Walks the tree and stops descending at the first match, so a `node_modules` inside a
/// `node_modules` is reported once, under its topmost parent.
#[must_use]
pub fn candidates(tree: &Tree, opts: &ClassifyOptions) -> Vec<Match> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = tree.node(tree.root).children.clone();

    while let Some(id) = stack.pop() {
        let node = tree.node(id);
        if node.missing || node.total_bytes < opts.min_bytes {
            continue;
        }
        if let Some(age) = opts.age {
            if !age.matches(node.newest_modified, opts.now) {
                // Still descend: a small old parent may hold a large matching child.
                stack.extend(node.children.iter().copied());
                continue;
            }
        }

        let path = tree.path_of(id);
        if let Some((kind, reason)) = rule_for(&path) {
            out.push(Match {
                node: id,
                path,
                bytes: node.total_bytes,
                newest_modified: node.newest_modified,
                kind: Some(kind),
                reason,
            });
            continue;
        }
        stack.extend(node.children.iter().copied());
    }

    out.sort_by_key(|m| std::cmp::Reverse(m.bytes));
    out
}

/// Which rule, if any, a path trips. Matched on the final component, with context where the
/// name alone is ambiguous.
fn rule_for(path: &std::path::Path) -> Option<(CandidateKind, String)> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();

    if name == ".trash" {
        return Some((CandidateKind::Trash, "the system trash".to_string()));
    }

    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            return Some((CandidateKind::Image, format!("a .{ext} disk image")));
        }
    }

    if REGENERABLE.contains(&name.as_str()) {
        return Some((
            CandidateKind::Regenerable,
            format!("build output or dependencies ({name}) that a toolchain recreates"),
        ));
    }

    if CACHES.contains(&name.as_str()) {
        return Some((CandidateKind::Cache, format!("a cache directory ({name})")));
    }

    if name.starts_with("backup") || name.contains(".backup") || name.contains("-backup") {
        return Some((
            CandidateKind::Backup,
            "a backup of something else".to_string(),
        ));
    }

    None
}

#[derive(Debug, Clone)]
pub struct FindQuery {
    pub min_bytes: u64,
    /// Either direction, or none.
    pub age: Option<AgeFilter>,
    /// Case-insensitive substring of the file name.
    pub name_contains: Option<String>,
    pub limit: usize,
}

impl Default for FindQuery {
    fn default() -> Self {
        Self {
            min_bytes: 0,
            age: None,
            name_contains: None,
            limit: 200,
        }
    }
}

/// Filter matches anywhere in the tree, with no opinion about what is deletable.
///
/// This is the answer to "I may well want to remove something recent": ask for it, rather
/// than having an age policy decide on your behalf.
#[must_use]
pub fn find(tree: &Tree, query: &FindQuery, now: u64) -> Vec<Match> {
    let needle = query.name_contains.as_ref().map(|s| s.to_lowercase());
    let mut out = Vec::new();

    for id in 0..tree.nodes.len() {
        if out.len() >= query.limit {
            break;
        }
        let node = tree.node(id);
        if id == tree.root || node.missing || node.total_bytes < query.min_bytes {
            continue;
        }
        if let Some(age) = query.age {
            if !age.matches(node.newest_modified, now) {
                continue;
            }
        }
        if let Some(needle) = &needle {
            if !node.name.to_lowercase().contains(needle) {
                continue;
            }
        }

        let mut reason = String::new();
        if let Some(age) = query.age {
            reason.push_str(&age.describe());
        }
        if !reason.is_empty() {
            reason.push_str(", ");
        }
        reason.push_str(&format!("at least {}", human(query.min_bytes)));

        out.push(Match {
            node: id,
            path: tree.path_of(id),
            bytes: node.total_bytes,
            newest_modified: node.newest_modified,
            kind: None,
            reason,
        });
    }

    out.sort_by_key(|m| std::cmp::Reverse(m.bytes));
    out
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
