//! The IPC surface the webview sees.
//!
//! Every command is deliberately narrow: the UI can read a tree, ask for a pruned view of
//! one node, add or remove collector items, and request the scan root's deletion. It can
//! never hand the backend a path to delete that did not come from a scan.

use crate::platform;
use crate::state::AppState;
use diskguardian_core::{
    build_view, delete, scan_with_progress, BranchDone, DeleteMode, ProgressFn, ViewNode,
    ViewOptions,
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
}

#[derive(Debug, Serialize)]
pub struct DeleteOutcomeView {
    pub path: String,
    pub ok: bool,
    pub bytes: u64,
    pub error: Option<String>,
}

fn lock_scans(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, HashMap<String, crate::state::Scan>>, String> {
    state
        .scans
        .lock()
        .map_err(|_| "scan state is poisoned by an earlier panic".to_string())
}

#[tauri::command]
pub fn home_dir() -> String {
    // `DISKGUARDIAN_ROOT` exists so a developer (or a test harness) can point the app at a
    // smaller tree without touching the default behaviour.
    std::env::var("DISKGUARDIAN_ROOT")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "/".to_string())
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

#[tauri::command]
pub async fn scan<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
) -> Result<ScanResponse, String> {
    let root = PathBuf::from(&path);
    if !root.exists() {
        return Err(format!("{path} does not exist"));
    }

    // Each branch is emitted the moment its subtree is measured, so the flower grows
    // instead of appearing at the end.
    let handle = app.clone();
    let progress: ProgressFn = Arc::new(move |branch: BranchDone| {
        let _ = handle.emit("scan-progress", &branch);
    });

    let scanned = tauri::async_runtime::spawn_blocking(move || {
        scan_with_progress(
            &root,
            &diskguardian_core::ScanOptions::default(),
            Some(progress),
        )
    })
    .await
    .map_err(|e| format!("scan task failed: {e}"))?
    .map_err(|e| e.to_string())?;

    let name = scanned.nodes[scanned.root].name.clone();
    let bytes = scanned.total_bytes();
    let files = scanned.file_count();
    let duration_ms = scanned.duration_ms;
    let errors = scanned.errors;
    let partial = scanned.nodes[scanned.root].partial;
    let view = build_view(&scanned, scanned.root, &ViewOptions::default());
    let scan_id = state.insert(scanned);

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
    })
}

#[tauri::command]
pub fn view(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
) -> Result<Vec<ViewNode>, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    let root = node_id.map_or(scan.tree.root, |id| id as usize);
    if root >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    Ok(build_view(&scan.tree, root, &ViewOptions::default()))
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

    Ok(FreshnessReport {
        removed,
        bytes: scan.tree.total_bytes(),
        files: scan.tree.file_count(),
        view: build_view(&scan.tree, root, &ViewOptions::default()),
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

    let view_root = node_id.map_or(scan.tree.root, |id| id as usize);
    let view = if view_root < scan.tree.nodes.len() {
        build_view(&scan.tree, view_root, &ViewOptions::default())
    } else {
        build_view(&scan.tree, scan.tree.root, &ViewOptions::default())
    };

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
