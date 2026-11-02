//! Deletion, behind a guard.
//!
//! The default is the macOS Trash: a user who collects the wrong thing can change
//! their mind. Permanent removal exists because emptying the Trash is itself a
//! step, but it is never the default.
//!
//! The guard is deliberately boring and deliberately absolute: system roots and the
//! scanned volume's own root are refused, whatever the UI believes.

use crate::model::{NodeId, Tree};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Above this size the UI must ask for explicit confirmation.
pub const CONFIRM_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Locations that are never deletable through this application.
///
/// Deliberately narrow. An earlier version protected `/private` and `/usr` wholesale,
/// which refused the user's own temp directory (`/private/var/folders`, through the
/// `/tmp` symlink) and every Homebrew cache (`/usr/local`) — a disk cleaner that cannot
/// clean the two places most likely to be full is a disk cleaner that lies. Protect the
/// system's own trees, not the prefixes they happen to live under.
///
/// Per-platform because the list is a property of the OS, not of this program: Windows
/// installs its system trees on an arbitrary drive letter under `%SystemRoot%`, and Linux
/// keeps them in different places again.
#[must_use]
pub fn protected_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        [
            "/System",
            "/usr/bin",
            "/usr/sbin",
            "/usr/lib",
            "/usr/libexec",
            "/usr/share",
            "/usr/standalone",
            "/bin",
            "/sbin",
            "/private/etc",
            "/private/var/db",
            "/private/var/vm",
            "/Library/Apple",
            "/dev",
            "/cores",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(windows)]
    {
        [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
        ]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        [
            "/bin",
            "/sbin",
            "/usr/bin",
            "/usr/sbin",
            "/usr/lib",
            "/lib",
            "/lib64",
            "/boot",
            "/etc",
            "/dev",
            "/proc",
            "/sys",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GuardError {
    #[error("refusing to delete {path}: it is a protected system location")]
    Protected { path: PathBuf },
    #[error("refusing to delete {path}: it is the root of the scanned volume")]
    ScanRoot { path: PathBuf },
    #[error("refusing to delete {path}: it is a filesystem root")]
    FilesystemRoot { path: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    /// Move to the macOS Trash (recoverable).
    Trash,
    /// Remove immediately, no recovery.
    Permanent,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteOutcome {
    pub path: PathBuf,
    pub ok: bool,
    pub bytes: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeleteReport {
    pub freed: u64,
    pub removed: u32,
    pub failed: u32,
    pub outcomes: Vec<DeleteOutcome>,
}

/// Validate a path against the guard, returning its absolute form.
pub fn guard(path: &Path, scan_root: &Path) -> Result<PathBuf, GuardError> {
    let absolute = fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    let root = fs::canonicalize(scan_root)
        .or_else(|_| std::path::absolute(scan_root))
        .unwrap_or_else(|_| scan_root.to_path_buf());

    if absolute == Path::new("/") {
        return Err(GuardError::FilesystemRoot { path: absolute });
    }
    if absolute == root {
        return Err(GuardError::ScanRoot { path: absolute });
    }
    for root in protected_roots() {
        if absolute == root || absolute.starts_with(&root) {
            return Err(GuardError::Protected { path: absolute });
        }
    }
    Ok(absolute)
}

#[must_use]
pub fn requires_confirmation(bytes: u64) -> bool {
    bytes >= CONFIRM_THRESHOLD_BYTES
}

/// Whether a deletion must be explicitly confirmed.
///
/// Two independent triggers, because size alone is the wrong test: a 200 MB `Desktop` is
/// still a whole top-level folder, and losing it is not the same as losing a cache.
///
/// 1. the collected bytes reach [`CONFIRM_THRESHOLD_BYTES`], or
/// 2. anything collected is a direct child of the scan root.
#[must_use]
pub fn needs_confirmation(tree: &Tree, ids: impl IntoIterator<Item = NodeId>, total: u64) -> bool {
    requires_confirmation(total)
        || ids
            .into_iter()
            .any(|id| tree.node(id).parent == Some(tree.root))
}

/// Delete each path, reporting per-item outcomes. Never panics on a bad path.
pub fn delete(paths: &[&Path], scan_root: &Path, mode: DeleteMode) -> DeleteReport {
    let mut report = DeleteReport::default();

    for path in paths {
        let absolute = match guard(path, scan_root) {
            Ok(p) => p,
            Err(err) => {
                report.failed += 1;
                report.outcomes.push(DeleteOutcome {
                    path: (*path).to_path_buf(),
                    ok: false,
                    bytes: 0,
                    error: Some(err.to_string()),
                });
                continue;
            }
        };

        let bytes = fs::symlink_metadata(&absolute).map_or(0, |m| {
            if m.is_dir() {
                // Directory output is the caller's already-computed tree size;
                // here we report what the filesystem tells us, which is its
                // metadata only.
                m.len()
            } else {
                crate::scan::allocation(&m)
            }
        });

        let result = match mode {
            DeleteMode::Trash => trash::delete(&absolute).map_err(|e| e.to_string()),
            DeleteMode::Permanent => remove(&absolute),
        };

        match result {
            Ok(()) => {
                report.removed += 1;
                report.freed = report.freed.saturating_add(bytes);
                report.outcomes.push(DeleteOutcome {
                    path: absolute,
                    ok: true,
                    bytes,
                    error: None,
                });
            }
            Err(error) => {
                report.failed += 1;
                report.outcomes.push(DeleteOutcome {
                    path: absolute,
                    ok: false,
                    bytes: 0,
                    error: Some(error),
                });
            }
        }
    }

    report
}

fn remove(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        fs::remove_file(path).map_err(|e| e.to_string())
    }
}
