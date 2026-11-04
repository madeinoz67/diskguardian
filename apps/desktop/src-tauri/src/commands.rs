//! The IPC surface the webview sees.
//!
//! Every command is deliberately narrow: the UI can read a tree, ask for a pruned view of
//! one node, add or remove collector items, and request the scan root's deletion. It can
//! never hand the backend a path to delete that did not come from a scan.

use crate::platform;
use crate::state::AppState;
use diskguardian_core::{
    build_view, build_volume_view, delete, scan_with_progress, BranchDone, DeleteMode, ProgressFn,
    ViewNode, ViewOptions, VolumeAnchor,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Emitter, State};

#[derive(Debug, Serialize)]
pub struct VolumeInfo {
    pub name: String,
    pub mount: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// Volume-level figures for a volume scan (C29), carried on every view-returning
/// response so the header renders without a panel call. `hidden_bytes` is
/// `used − tree`, signed: negative is a state ("the disk changed during or after the
/// scan") for words, not a petal.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct VolumeSummary {
    pub used_bytes: u64,
    pub hidden_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct ScanResponse {
    pub scan_id: String,
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
    pub duration_ms: u64,
    pub errors: u64,
    pub partial: bool,
    pub view: Vec<ViewNode>,
    /// Present iff the scan root was a mount point.
    pub volume: Option<VolumeSummary>,
}

#[derive(Debug, Serialize)]
pub struct CollectState {
    pub total: u64,
    pub count: usize,
    pub ids: Vec<u64>,
    /// Computed here, not in the UI: a top-level folder asks for confirmation whatever its
    /// size, and the frontend must not be the thing that decides that.
    pub needs_confirmation: bool,
}

#[derive(Debug, Serialize)]
pub struct DeleteReport {
    pub freed: u64,
    pub removed: u32,
    pub failed: u32,
    pub mode: String,
    pub outcomes: Vec<DeleteOutcomeView>,
    /// The view to render next, pruned of what was deleted. Refreshing this way avoids
    /// re-walking the volume, which on a large disk takes minutes.
    pub view: Vec<ViewNode>,
    pub bytes: u64,
    pub files: u64,
    pub volume: Option<VolumeSummary>,
}

#[derive(Debug, Serialize)]
pub struct DeleteOutcomeView {
    pub path: String,
    pub ok: bool,
    pub bytes: u64,
    pub error: Option<String>,
}

/// The scans table, or why it could not be taken. `pub(crate)` so the favourites
/// commands can resolve node paths through the same seam `reveal` uses.
pub(crate) fn lock_scans(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, HashMap<String, crate::state::Scan>>, String> {
    state
        .scans
        .lock()
        .map_err(|_| "scan state is poisoned by an earlier panic".to_string())
}

/// Resolve the home directory from an environment lookup, without touching the process
/// environment. Split out from `home_dir` so both platforms are testable on one machine:
/// the platform is a parameter here, not a compile-time accident.
fn home_from(lookup: impl Fn(&str) -> Option<String>, is_windows: bool) -> String {
    // An empty variable is not a path. `HOME=""` is common under cron and some launchers,
    // and returning it verbatim would point the scan at nothing while looking like a
    // successful scan of an empty disk.
    let non_empty = |key: &str| lookup(key).filter(|value| !value.is_empty());

    // `DISKGUARDIAN_ROOT` exists so a developer (or a test harness) can point the app at a
    // smaller tree without touching the default behaviour.
    if let Some(root) = non_empty("DISKGUARDIAN_ROOT") {
        return root;
    }

    if is_windows {
        // Windows sets `USERPROFILE`. `HOME` is a POSIX variable it does not set, which is
        // why this used to fall through to `/`. `HOMEDRIVE`+`HOMEPATH` is the older pair,
        // still present where `USERPROFILE` is redirected (roaming or domain profiles).
        if let Some(profile) = non_empty("USERPROFILE") {
            return profile;
        }
        if let (Some(drive), Some(path)) = (non_empty("HOMEDRIVE"), non_empty("HOMEPATH")) {
            return format!("{drive}{path}");
        }
        // `/` is a real path on Unix and means nothing on Windows, so a Windows build with
        // no home variables still names a Windows path.
        return "C:\\".to_string();
    }

    non_empty("HOME").unwrap_or_else(|| "/".to_string())
}

#[tauri::command]
pub fn home_dir() -> String {
    home_from(|key| std::env::var(key).ok(), cfg!(windows))
}

/// Mounted volumes, cross-platform.
///
/// This used to shell out to `df -k`, which is Unix-only. `sysinfo` asks each platform's own
/// API instead: `getmntinfo` on macOS, `/proc/mounts` on Linux, `GetLogicalDrives` on
/// Windows. It also means no external command has to exist.
#[tauri::command]
pub fn volumes() -> Vec<VolumeInfo> {
    let disks = sysinfo::Disks::new_with_refreshed_list();

    let mut volumes: Vec<VolumeInfo> = disks
        .iter()
        .filter(|disk| disk.total_space() > 0)
        .map(|disk| {
            let mount = disk.mount_point().to_string_lossy().into_owned();
            let label = disk.name().to_string_lossy().into_owned();
            VolumeInfo {
                name: if label.is_empty() && mount == "/" {
                    "Startup disk".to_string()
                } else if label.is_empty() {
                    mount.clone()
                } else {
                    label
                },
                mount,
                total_bytes: disk.total_space(),
                free_bytes: disk.available_space(),
            }
        })
        .collect();

    // Biggest first, which is the disk people are usually worried about.
    volumes.sort_by_key(|v| std::cmp::Reverse(v.total_bytes));
    volumes
}

/// The names of a directory's immediate children, so the UI can draw placeholder petals
/// before any sizes are known. One `readdir`, no walking.
#[tauri::command]
pub fn peek_root(path: String) -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(&path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read_dir
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// One branch finishing, stamped with the scan that finished it.
///
/// The stamp is the point: `scan-progress` is one global event name, so without it the next
/// scan's listener accepts the previous scan's reports and paints its petals with somebody
/// else's numbers. A scan id would not do, because it is minted only when the walk finishes;
/// the frontend's per-start nonce exists before the walk does.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanProgress {
    pub nonce: String,
    pub name: String,
    pub bytes: u64,
    pub files: u64,
}

/// The words that ride any negative residual, so the panel can never show one as a
/// silent zero.
pub const NEGATIVE_RESIDUAL_NOTE: &str =
    "The volume changed more than the scan window — the disk moved during or after this \
     scan. Rescan for exact figures.";

/// Purgeable, worded as what it is: an upper bound beside the figure, never a
/// component of the hidden figure (the system may free space the tree already counts).
pub const PURGEABLE_NOTE: &str =
    "up to this much: snapshots, caches and offloaded content macOS may free on its \
     own. Not part of the hidden figure — much of it is space the scan already counts. \
     Measured as the difference between two capacity figures the OS reports";

/// The decomposition into other container volumes, the snapshot count, protected areas
/// and a still-hidden remainder is **held**, and this is the panel's wording for that
/// state. The pre-committed acceptance rule (design §7) marked its plausibility arm
/// underpowered — still-hidden was 92.3% of the hidden figure, above the rule's own 80%
/// line — and the external-volume tie-breaker has not run, so no category is shown:
/// neither as a figure nor as a zero. The residual itself ships (it is the one number
/// the evidence supports). Un-holding means running that cross-check and restoring the
/// measurement; the parsers stay in [`crate::volume`] for exactly that.
pub const DECOMPOSITION_HELD_NOTE: &str =
    "The breakdown into other volumes, snapshots and protected areas is held pending a \
     cross-check on a second volume: until then no category of the hidden figure is \
     shown — not as a figure, and not as zero. The hidden figure itself is measured: \
     the volume's used space minus everything this scan could see.";

/// Project a view of `scan` at `node_id` (None: the scan root). For a volume scan's
/// root view the anchor is re-measured now (one statfs) and the hidden petal is drawn
/// from the *current* tree, so deletes, grafts and revalidates move it for free. An
/// anchor that cannot be measured yields the plain view and no summary — never a
/// hidden node anchored at zero.
pub(crate) fn project(
    state: &AppState,
    scan: &crate::state::Scan,
    node_id: Option<u64>,
) -> Result<(Vec<ViewNode>, Option<VolumeSummary>), String> {
    let root = node_id.map_or(scan.tree.root, |id| id as usize);
    if root >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    match &scan.volume {
        // The anchor is a volume-level figure: only the scan root's own projection
        // carries the petal. Deeper views are ordinary tree views.
        Some(ctx) if root == scan.tree.root => {
            let Some(used) = crate::volume::probed_volume_used(state, &ctx.mount) else {
                return Ok((build_view(&scan.tree, root, &ViewOptions::default()), None));
            };
            let anchor = VolumeAnchor { used_bytes: used };
            let view = build_volume_view(&scan.tree, root, &ViewOptions::default(), &anchor);
            Ok((
                view.nodes,
                Some(VolumeSummary {
                    used_bytes: used,
                    hidden_bytes: view.residual_bytes,
                }),
            ))
        }
        _ => Ok((build_view(&scan.tree, root, &ViewOptions::default()), None)),
    }
}

#[tauri::command]
pub async fn scan<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
    nonce: Option<String>,
) -> Result<ScanResponse, String> {
    let root = PathBuf::from(&path);
    if !root.exists() {
        return Err(format!("{path} does not exist"));
    }
    let nonce = nonce.unwrap_or_default();

    // A volume scan is a scan whose root is a mount point. It is bounded by device (the
    // APFS boot group presents as one) and, on macOS, by the two fixed prefixes that
    // share the boot group's device id — without which the walk would enter the Data
    // volume twice through the merged namespace and count directory metadata twice.
    // A folder scan changes nothing: no anchor, no boundary, no hidden node.
    let is_volume_scan = crate::volume::is_mount_point(&root);

    // The walk in flight watches this flag, and `cancel_scan` is what sets it. Registering a
    // new scan *replaces* the slot rather than signalling the old walk — so a walk is stopped
    // only if a cancel lands before this line runs. The comments here used to claim otherwise,
    // which read fine and was false; see ISA R14 for what that ordering cannot express.
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let clear_me = cancel.clone();
    {
        let mut slot = state
            .scan_cancel
            .lock()
            .map_err(|_| "scan state is poisoned by an earlier panic".to_string())?;
        *slot = Some(cancel.clone());
    }

    // Each branch is emitted the moment its subtree is measured, so the flower grows
    // instead of appearing at the end — and stamped, so only the scan that owns this walk
    // can act on it.
    let handle = app.clone();
    let stamp = nonce.clone();
    let progress: ProgressFn = Arc::new(move |branch: BranchDone| {
        let _ = handle.emit(
            "scan-progress",
            &ScanProgress {
                nonce: stamp.clone(),
                name: branch.name,
                bytes: branch.bytes,
                files: branch.files,
            },
        );
    });

    let boundary_prefixes = if is_volume_scan {
        crate::volume::volume_scan_exclusions()
    } else {
        Vec::new()
    };
    let walk_root = root.clone();
    let bounded = is_volume_scan;
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let options = diskguardian_core::ScanOptions {
            cancel: Some(cancel),
            same_device_only: bounded,
            exclude_prefixes: boundary_prefixes,
            ..Default::default()
        };
        scan_with_progress(&walk_root, &options, Some(progress))
    })
    .await
    .map_err(|e| format!("scan task failed: {e}"))?;

    // Retire the flag. Not because a leftover would cancel the next scan — the next scan
    // overwrites the slot with a fresh flag and a walk only ever reads its own — but so a
    // cancel arriving after this walk ended does not signal a finished scan.
    if let Ok(mut slot) = state.scan_cancel.lock() {
        if slot
            .as_ref()
            .is_some_and(|flag| Arc::ptr_eq(flag, &clear_me))
        {
            *slot = None;
        }
    }

    let scanned = outcome.map_err(|e| e.to_string())?;

    // The scan-time measurement (the purgeable annotation) is taken once, here; the
    // anchor, by contrast, is re-measured at every projection (§ `project`). The
    // decomposition is not measured at all while it is held — see
    // [`DECOMPOSITION_HELD_NOTE`].
    let volume_ctx = if is_volume_scan {
        Some(measure_volume_context(&state, &root).await?)
    } else {
        None
    };

    let name = scanned.nodes[scanned.root].name.clone();
    let bytes = scanned.total_bytes();
    let files = scanned.file_count();
    let duration_ms = scanned.duration_ms;
    let errors = scanned.errors;
    let partial = scanned.nodes[scanned.root].partial;
    let (view, volume) = match &volume_ctx {
        Some(ctx) => match crate::volume::probed_volume_used(&state, &ctx.mount) {
            Some(used) => {
                let anchor = VolumeAnchor { used_bytes: used };
                let projected =
                    build_volume_view(&scanned, scanned.root, &ViewOptions::default(), &anchor);
                (
                    projected.nodes,
                    Some(VolumeSummary {
                        used_bytes: used,
                        hidden_bytes: projected.residual_bytes,
                    }),
                )
            }
            None => (
                build_view(&scanned, scanned.root, &ViewOptions::default()),
                None,
            ),
        },
        None => (
            build_view(&scanned, scanned.root, &ViewOptions::default()),
            None,
        ),
    };
    let scan_id = state.insert_with(scanned, volume_ctx);

    Ok(ScanResponse {
        scan_id,
        name,
        path,
        bytes,
        files,
        duration_ms,
        errors,
        partial,
        view,
        volume,
    })
}

/// The scan-time half of hidden space, measured once per volume scan. Only the
/// purgeable annotation is measured here: the decomposition (other container volumes,
/// the snapshot count, the protected areas) is held — [`DECOMPOSITION_HELD_NOTE`] is the
/// panel's words for that state, and its parsers stay in `volume.rs` for the
/// cross-check that can un-hold it. A probe seam injected by the tests stands in for
/// the OS call.
async fn measure_volume_context(
    state: &AppState,
    mount: &Path,
) -> Result<crate::state::VolumeScan, String> {
    let seam = state
        .volume_probes
        .lock()
        .map_err(|_| "volume probe seam is poisoned".to_string())?
        .clone();

    let purgeable = match &seam {
        Some(probes) => (probes.purgeable)(mount),
        None => crate::volume::purgeable_bytes(mount),
    };

    let measured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(crate::state::VolumeScan {
        mount: mount.to_path_buf(),
        purgeable,
        measured_at,
    })
}

/// Ask the walk in flight to stop. The idempotent no-op case matters: the frontend calls this
/// before every scan, including the first one of an app run.
#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) -> Result<(), String> {
    let slot = state
        .scan_cancel
        .lock()
        .map_err(|_| "scan state is poisoned by an earlier panic".to_string())?;
    if let Some(flag) = slot.as_ref() {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub fn view(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
) -> Result<Vec<ViewNode>, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    Ok(project(&state, scan, node_id)?.0)
}

#[derive(Debug, Serialize)]
pub struct CleanupItem {
    pub node: u64,
    pub path: String,
    pub bytes: u64,
    pub newest_modified: Option<u64>,
    /// Rule label ("regenerable", "cache", ...) or absent for a size-only match.
    pub kind: Option<String>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct CleanupReport {
    /// Things a rule recognises as rebuildable or disposable, biggest first.
    pub candidates: Vec<CleanupItem>,
    /// The largest items in the scan regardless of rule, biggest first.
    pub biggest: Vec<CleanupItem>,
    /// Same size floor, but the other side of the age bound, when the caller asked for it.
    pub recent: Vec<CleanupItem>,
    pub total_candidate_bytes: u64,
}

fn to_item(m: &diskguardian_core::Match) -> CleanupItem {
    CleanupItem {
        node: m.node as u64,
        path: m.path.display().to_string(),
        bytes: m.bytes,
        newest_modified: m.newest_modified,
        kind: m.kind.map(|k| k.label().to_string()),
        reason: m.reason.clone(),
    }
}

/// What is filling the disk, and what looks disposable.
///
/// Read-only: this proposes, and the human still collects and deletes. Age is a parameter in
/// both directions rather than a policy, because "recent but unwanted" is a real question.
#[tauri::command]
pub fn cleanup(
    state: State<'_, AppState>,
    scan_id: String,
    min_bytes: u64,
    age_days: Option<u64>,
    newer: bool,
) -> Result<CleanupReport, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let age = age_days.map(|days| {
        if newer {
            diskguardian_core::AgeFilter::NewerThanDays(days)
        } else {
            diskguardian_core::AgeFilter::OlderThanDays(days)
        }
    });

    let found = diskguardian_core::candidates(
        &scan.tree,
        &diskguardian_core::ClassifyOptions {
            min_bytes,
            age,
            now,
        },
    );

    let biggest = diskguardian_core::find(
        &scan.tree,
        &diskguardian_core::FindQuery {
            min_bytes,
            age: None,
            name_contains: None,
            limit: 50,
        },
        now,
    );

    // The same floor on the other side of the age bound: "recent and large" is a legitimate
    // question, and the caller should not have to run two scans to ask it.
    let recent = if age_days.is_some() {
        let opposite = age.map(|_| {
            if newer {
                diskguardian_core::AgeFilter::OlderThanDays(0)
            } else {
                diskguardian_core::AgeFilter::NewerThanDays(0)
            }
        });
        diskguardian_core::find(
            &scan.tree,
            &diskguardian_core::FindQuery {
                min_bytes,
                age: opposite,
                name_contains: None,
                limit: 25,
            },
            now,
        )
    } else {
        Vec::new()
    };

    Ok(CleanupReport {
        total_candidate_bytes: found.iter().map(|m| m.bytes).sum(),
        candidates: found.iter().map(to_item).collect(),
        biggest: biggest.iter().map(to_item).collect(),
        recent: recent.iter().map(to_item).collect(),
    })
}

/// How long a duplicates pass may run before it reports what it has. A click on a big
/// tree must come back with an answer and a "not finished" flag, not spin.
const DUPLICATE_BUDGET_MS: u64 = 8_000;

/// Content-identical files in a finished scan.
///
/// Read-only: this proposes, the collector disposes. Nothing here deletes, and the report
/// carries the engine's own caveat so a group of APFS clones can never be shown as
/// guaranteed free space.
///
/// `async` matters: a whole-file hash over a large group is seconds of I/O, and a sync
/// command runs on the UI thread. The scan mutex is held for the pass, so other
/// scan-scoped commands wait for it — the window itself keeps painting.
#[tauri::command(async)]
pub fn duplicates(
    state: State<'_, AppState>,
    scan_id: String,
    min_bytes: u64,
    budget_ms: Option<u64>,
) -> Result<diskguardian_core::DuplicateReport, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;

    let mut options = diskguardian_core::DuplicateOptions::new(min_bytes);
    options.deadline = Some(
        std::time::Instant::now()
            + std::time::Duration::from_millis(budget_ms.unwrap_or(DUPLICATE_BUDGET_MS)),
    );

    diskguardian_core::find_duplicates(&scan.tree, &options).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct TrashStats {
    /// Where this platform keeps the trash, empty when it has no path-based one.
    pub path: String,
    pub bytes: u64,
    pub items: u64,
    pub supported: bool,
}

#[derive(Debug, Serialize)]
pub struct EmptyReport {
    pub freed: u64,
    pub removed: u64,
    pub failed: u64,
}

/// The trash this platform keeps inside the user's home, if it has one.
fn trash_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Trash"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            return Some(PathBuf::from(data).join("Trash"));
        }
        return std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/Trash"));
    }

    #[cfg(windows)]
    {
        None
    }
}

/// How much the trash is holding. Emptying it is the step that actually returns the space.
#[tauri::command]
pub fn trash_stats() -> TrashStats {
    let Some(path) = trash_path() else {
        return TrashStats {
            path: String::new(),
            bytes: 0,
            items: 0,
            supported: cfg!(windows),
        };
    };

    // Linux keeps the payload under `files/`, macOS directly in `.Trash`.
    let payload = if path.join("files").is_dir() {
        path.join("files")
    } else {
        path.clone()
    };

    let mut bytes = 0u64;
    let mut items = 0u64;
    if let Ok(entries) = std::fs::read_dir(&payload) {
        for entry in entries.flatten() {
            items += 1;
            bytes = bytes.saturating_add(walk_bytes(&entry.path()));
        }
    }

    TrashStats {
        path: payload.display().to_string(),
        bytes,
        items,
        supported: true,
    }
}

fn walk_bytes(path: &std::path::Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_dir() {
        let mut total = diskguardian_core::scan::allocation(&meta);
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                total = total.saturating_add(walk_bytes(&entry.path()));
            }
        }
        total
    } else {
        diskguardian_core::scan::allocation(&meta)
    }
}

/// Empty the trash. Irreversible, so it always needs `confirmed`.
#[tauri::command]
pub fn empty_trash(confirmed: bool) -> Result<EmptyReport, String> {
    if !confirmed {
        return Err("emptying the trash needs explicit confirmation".to_string());
    }

    #[cfg(windows)]
    {
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
            ])
            .status()
            .map_err(|e| format!("could not run powershell: {e}"))?;
        return Ok(EmptyReport {
            freed: 0,
            removed: 0,
            failed: if status.success() { 0 } else { 1 },
        });
    }

    #[cfg(not(windows))]
    {
        let Some(path) = trash_path() else {
            return Err("no trash directory on this platform".to_string());
        };
        let payload = if path.join("files").is_dir() {
            path.join("files")
        } else {
            path
        };

        let mut freed = 0u64;
        let mut removed = 0u64;
        let mut failed = 0u64;

        let entries = std::fs::read_dir(&payload).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let target = entry.path();
            let size = walk_bytes(&target);
            let result = if target.is_dir() {
                std::fs::remove_dir_all(&target)
            } else {
                std::fs::remove_file(&target)
            };
            match result {
                Ok(()) => {
                    freed = freed.saturating_add(size);
                    removed += 1;
                }
                Err(_) => failed += 1,
            }
        }

        // Linux keeps matching metadata beside the payload.
        let info = payload.parent().map(|p| p.join("info"));
        if let Some(info) = info.filter(|i| i.is_dir()) {
            if let Ok(entries) = std::fs::read_dir(info) {
                for entry in entries.flatten() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }

        Ok(EmptyReport {
            freed,
            removed,
            failed,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct FreshnessReport {
    /// How many nodes in this view no longer exist on disk.
    pub removed: u32,
    pub bytes: u64,
    pub files: u64,
    pub view: Vec<ViewNode>,
    pub volume: Option<VolumeSummary>,
}

/// Drop view nodes whose paths have disappeared since the scan.
///
/// The tree is a snapshot, so anything deleted outside the app keeps being drawn. This
/// re-stats the current level only, which is a handful of metadata calls, and prunes what is
/// gone: cheap enough to run whenever the window regains focus.
#[tauri::command]
pub fn revalidate(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
) -> Result<FreshnessReport, String> {
    let mut scans = lock_scans(&state)?;
    let scan = scans.get_mut(&scan_id).ok_or("unknown scan")?;

    let root = node_id.map_or(scan.tree.root, |id| id as usize);
    if root >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }

    let children: Vec<usize> = scan.tree.node(root).children.clone();
    let mut gone: Vec<usize> = Vec::new();
    for id in children {
        if !scan.tree.path_of(id).exists() {
            gone.push(id);
        }
    }

    let removed = gone.len() as u32;
    if removed > 0 {
        scan.tree.mark_missing(&gone);
    }

    let (view, volume) = project(&state, scan, node_id)?;
    Ok(FreshnessReport {
        removed,
        bytes: scan.tree.total_bytes(),
        files: scan.tree.file_count(),
        view,
        volume,
    })
}

#[tauri::command]
pub fn collect(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u64,
    add: bool,
) -> Result<CollectState, String> {
    let mut scans = lock_scans(&state)?;
    let scan = scans.get_mut(&scan_id).ok_or("unknown scan")?;
    let node = node_id as usize;
    if node >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    if scan.tree.node(node).kind == diskguardian_core::NodeKind::Symlink {
        // A symlink is a pointer someone placed on purpose. Deleting one costs nothing and
        // breaks whatever it points at, so it never enters the collector.
        return Err("symbolic links are not collected: they may be there for a reason".to_string());
    }
    if node == scan.tree.root {
        // The guard refuses to delete the scan root, so collecting it would only build a
        // collector that cannot be emptied. Fail where the user can still see why.
        return Err("the scanned folder itself cannot be collected".to_string());
    }
    if add {
        scan.collector.add(node);
    } else {
        scan.collector.remove(node);
    }
    Ok(collect_state(scan))
}

fn collect_state(scan: &crate::state::Scan) -> CollectState {
    let total = scan.collector.total(&scan.tree);
    let ids: Vec<usize> = scan.collector.deletion_order(&scan.tree);
    CollectState {
        total,
        count: scan.collector.len(),
        ids: scan.collector.ids().map(|id| id as u64).collect(),
        needs_confirmation: diskguardian_core::delete::needs_confirmation(&scan.tree, ids, total),
    }
}

#[tauri::command]
pub fn collected(state: State<'_, AppState>, scan_id: String) -> Result<CollectState, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    Ok(collect_state(scan))
}

#[tauri::command]
pub fn delete_collected(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
    permanent: bool,
    confirmed: bool,
) -> Result<DeleteReport, String> {
    let mut scans = lock_scans(&state)?;
    let scan = scans.get_mut(&scan_id).ok_or("unknown scan")?;

    let total = scan.collector.total(&scan.tree);
    if total == 0 {
        return Err("the collector is empty".to_string());
    }
    let order = scan.collector.deletion_order(&scan.tree);
    if diskguardian_core::delete::needs_confirmation(&scan.tree, order.clone(), total) && !confirmed
    {
        return Err(format!(
            "deleting {total} bytes needs explicit confirmation (top-level folder or over the size bound)"
        ));
    }

    // Align outcomes to ids by position, not by path.
    //
    // `delete` returns one outcome per path, in the order given, so position is exact.
    // Matching on the path looked equivalent and was not: `guard` canonicalises, so on
    // macOS `/var/...` comes back as `/private/var/...`, the lookup missed, nothing was
    // pruned, and the "freed" figure was the directory's metadata size rather than its
    // contents.
    let paths: Vec<PathBuf> = order.iter().map(|&id| scan.tree.path_of(id)).collect();

    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let mode = if permanent {
        DeleteMode::Permanent
    } else {
        DeleteMode::Trash
    };
    let outcome = delete(&refs, &scan.scan_root, mode);

    let mut deleted: Vec<usize> = Vec::new();
    let mut freed = 0u64;
    for (&id, result) in order.iter().zip(outcome.outcomes.iter()) {
        if result.ok {
            freed = freed.saturating_add(scan.tree.node(id).total_bytes);
            deleted.push(id);
        }
    }

    // Prune what actually left the disk, then re-project the view. No rescan.
    scan.tree.mark_missing(&deleted);
    scan.collector.clear();

    let view_root = node_id.filter(|&id| (id as usize) < scan.tree.nodes.len());
    let (view, volume) = project(&state, scan, view_root)?;

    Ok(DeleteReport {
        freed,
        removed: outcome.removed,
        failed: outcome.failed,
        mode: if permanent {
            "permanent".to_string()
        } else {
            "trash".to_string()
        },
        outcomes: outcome
            .outcomes
            .into_iter()
            .map(|o| DeleteOutcomeView {
                path: o.path.display().to_string(),
                ok: o.ok,
                bytes: o.bytes,
                error: o.error,
            })
            .collect(),
        bytes: scan.tree.total_bytes(),
        files: scan.tree.file_count(),
        view,
        volume,
    })
}

// ─── hidden space (C29) ───────────────────────────────────────────────────────────────

/// The hidden-space report of one volume scan. The residual is the shipped figure;
/// the decomposition is held ([`DECOMPOSITION_HELD_NOTE`]) and therefore absent from
/// this payload entirely — a category that is not in the payload cannot be rendered as
/// a figure or as a zero, which is the exact dishonesty the held state exists to
/// prevent. `purgeable_bytes: None` is unmeasured, never zero; `negative_note` rides
/// any negative residual so it can never be drawn as a silent zero.
#[derive(Debug, Serialize)]
pub struct HiddenSpaceReport {
    pub mount: String,
    /// The volume's used space, measured now: the anchor.
    pub used_bytes: u64,
    pub tree_bytes: u64,
    /// `used − tree`, signed and unclamped.
    pub hidden_bytes: i64,
    /// The held state's words, verbatim: the panel passes them through.
    pub decomposition_held_note: String,
    pub purgeable_bytes: Option<u64>,
    pub purgeable_note: Option<String>,
    /// The mounts the volume boundary did not enter, listed rather than silently
    /// skipped.
    pub skipped_mounts: Vec<String>,
    /// Unix seconds of the scan-time measurement (the purgeable annotation), for the
    /// "measured at scan time" line.
    pub measured_at: u64,
    pub negative_note: Option<String>,
}

/// The report behind the hidden petal. The anchor and the residual are measured now,
/// fresh at every call; the purgeable annotation is the once-per-scan measurement the
/// scan carries. The webview supplies a scan id and nothing else.
#[tauri::command]
pub fn hidden_space(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<HiddenSpaceReport, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    let ctx = scan.volume.as_ref().ok_or(
        "this scan is not a volume scan: hidden space applies to a scanned disk, not a folder",
    )?;

    let used = crate::volume::probed_volume_used(&state, &ctx.mount).ok_or(
        "the volume's used space could not be measured right now — the mount may have \
         gone; rescan"
            .to_string(),
    )?;
    let tree_bytes = scan.tree.total_bytes();
    let hidden =
        (used as i128 - tree_bytes as i128).clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    let purgeable = ctx.purgeable;

    Ok(HiddenSpaceReport {
        mount: ctx.mount.display().to_string(),
        used_bytes: used,
        tree_bytes,
        hidden_bytes: hidden,
        decomposition_held_note: DECOMPOSITION_HELD_NOTE.to_string(),
        purgeable_bytes: purgeable,
        purgeable_note: purgeable.map(|_| PURGEABLE_NOTE.to_string()),
        skipped_mounts: scan.tree.skipped_mounts.clone(),
        measured_at: ctx.measured_at,
        negative_note: (hidden < 0).then(|| NEGATIVE_RESIDUAL_NOTE.to_string()),
    })
}

fn node_path(state: &AppState, scan_id: &str, node_id: u64) -> Result<PathBuf, String> {
    let scans = lock_scans(state)?;
    let scan = scans.get(scan_id).ok_or("unknown scan")?;
    let node = node_id as usize;
    if node >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    Ok(scan.tree.path_of(node))
}

#[tauri::command]
pub fn reveal(state: State<'_, AppState>, scan_id: String, node_id: u64) -> Result<(), String> {
    let path = node_path(&state, &scan_id, node_id)?;
    platform::reveal(&path)
}

#[tauri::command]
pub fn quicklook(state: State<'_, AppState>, scan_id: String, node_id: u64) -> Result<(), String> {
    let path = node_path(&state, &scan_id, node_id)?;
    platform::quicklook(&path)
}

#[cfg(test)]
mod tests {
    use super::home_from;

    /// A stand-in for the process environment: only the pairs given exist.
    fn env(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let owned = pairs.to_vec();
        move |key| {
            owned
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    // The bug this guards: Windows does not set `HOME`, so reading only `HOME` fell
    // through to `/` and the overview offered the filesystem root instead of the user's
    // own directory. `USERPROFILE` is the variable Windows actually sets.

    #[test]
    fn windows_uses_userprofile() {
        let home = home_from(
            env(&[("USERPROFILE", "C:\\Users\\seaton"), ("HOME", "")]),
            true,
        );
        assert_eq!(home, "C:\\Users\\seaton");
    }

    #[test]
    fn windows_falls_back_to_homedrive_and_homepath() {
        let home = home_from(
            env(&[("HOMEDRIVE", "D:"), ("HOMEPATH", "\\Profiles\\seaton")]),
            true,
        );
        assert_eq!(home, "D:\\Profiles\\seaton");
    }

    #[test]
    fn windows_never_falls_back_to_the_unix_root() {
        // `/` is a real path on Unix and means nothing on Windows. A Windows build with
        // no home variables must still name a Windows path.
        let home = home_from(env(&[]), true);
        assert_ne!(home, "/");
        assert_eq!(home, "C:\\");
    }

    #[test]
    fn unix_uses_home() {
        let home = home_from(env(&[("HOME", "/Users/seaton")]), false);
        assert_eq!(home, "/Users/seaton");
    }

    #[test]
    fn unix_falls_back_to_the_root() {
        let home = home_from(env(&[]), false);
        assert_eq!(home, "/");
    }

    #[test]
    fn root_override_wins_on_both_platforms() {
        for is_windows in [true, false] {
            let home = home_from(
                env(&[
                    ("DISKGUARDIAN_ROOT", "/tmp/fixture"),
                    ("USERPROFILE", "C:\\Users\\seaton"),
                    ("HOME", "/Users/seaton"),
                ]),
                is_windows,
            );
            assert_eq!(home, "/tmp/fixture");
        }
    }

    #[test]
    fn empty_variables_are_not_a_home() {
        // An empty HOME is set often enough (cron, some launchers) that treating it as a
        // real path would scan nothing and look like an empty disk.
        let home = home_from(env(&[("HOME", ""), ("USERPROFILE", "")]), false);
        assert_eq!(home, "/");
    }
}
