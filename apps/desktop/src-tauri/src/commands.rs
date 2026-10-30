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
use std::sync::Arc;
use std::path::{Path, PathBuf};
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
}

#[derive(Debug, Serialize)]
pub struct DeleteReport {
    pub freed: u64,
    pub removed: u32,
    pub failed: u32,
    pub outcomes: Vec<DeleteOutcomeView>,
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

/// Mounted volumes, via `df`. Deliberately not a sysinfo dependency: one call, parsed
/// once, on a code path that never touches the scan results.
#[tauri::command]
pub fn volumes() -> Vec<VolumeInfo> {
    let output = std::process::Command::new("df").args(["-k", "-P"]).output();
    let Ok(output) = output else {
        return Vec::new();
    };
    parse_df(&String::from_utf8_lossy(&output.stdout))
}

fn parse_df(text: &str) -> Vec<VolumeInfo> {
    let mut volumes = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 6 {
            continue;
        }
        let mount = fields[5..].join(" ");
        if !mount.starts_with('/') {
            continue;
        }
        let filesystem = fields[0];
        if filesystem.starts_with("devfs") || filesystem.starts_with("map") {
            continue;
        }
        let Ok(blocks) = fields[1].parse::<u64>() else {
            continue;
        };
        let Ok(available) = fields[3].parse::<u64>() else {
            continue;
        };
        let name = if mount == "/" {
            "Macintosh HD".to_string()
        } else {
            Path::new(&mount)
                .file_name()
                .map_or_else(|| mount.clone(), |n| n.to_string_lossy().into_owned())
        };
        volumes.push(VolumeInfo {
            name,
            mount,
            total_bytes: blocks.saturating_mul(1024),
            free_bytes: available.saturating_mul(1024),
        });
    }
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
pub async fn scan(
    app: tauri::AppHandle,
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
        scan_with_progress(&root, &diskguardian_core::ScanOptions::default(), Some(progress))
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
    if add {
        scan.collector.add(node);
    } else {
        scan.collector.remove(node);
    }
    Ok(CollectState {
        total: scan.collector.total(&scan.tree),
        count: scan.collector.len(),
        ids: scan.collector.ids().map(|id| id as u64).collect(),
    })
}

#[tauri::command]
pub fn collected(state: State<'_, AppState>, scan_id: String) -> Result<CollectState, String> {
    let scans = lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    Ok(CollectState {
        total: scan.collector.total(&scan.tree),
        count: scan.collector.len(),
        ids: scan.collector.ids().map(|id| id as u64).collect(),
    })
}

#[tauri::command]
pub fn delete_collected(
    state: State<'_, AppState>,
    scan_id: String,
    permanent: bool,
    confirmed: bool,
) -> Result<DeleteReport, String> {
    let mut scans = lock_scans(&state)?;
    let scan = scans.get_mut(&scan_id).ok_or("unknown scan")?;

    let total = scan.collector.total(&scan.tree);
    if total == 0 {
        return Err("the collector is empty".to_string());
    }
    if diskguardian_core::delete::requires_confirmation(total) && !confirmed {
        return Err(format!(
            "deleting {total} bytes needs explicit confirmation"
        ));
    }

    let order = scan.collector.deletion_order(&scan.tree);
    let mut sizes: HashMap<PathBuf, u64> = HashMap::new();
    let mut paths: Vec<PathBuf> = Vec::with_capacity(order.len());
    for id in order {
        let path = scan.tree.path_of(id);
        sizes.insert(path.clone(), scan.tree.node(id).total_bytes);
        paths.push(path);
    }

    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let mode = if permanent {
        DeleteMode::Permanent
    } else {
        DeleteMode::Trash
    };
    let outcome = delete(&refs, &scan.scan_root, mode);

    let freed: u64 = outcome
        .outcomes
        .iter()
        .filter(|o| o.ok)
        .map(|o| sizes.get(&o.path).copied().unwrap_or(o.bytes))
        .sum();

    scan.collector.clear();

    Ok(DeleteReport {
        freed,
        removed: outcome.removed,
        failed: outcome.failed,
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
    use super::parse_df;

    #[test]
    fn parses_df_output_including_mounts_with_spaces() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                    /dev/disk3s1s1 971350180 10485760 500000000 3% /\n\
                    devfs 200 200 0 100% /dev\n\
                    /dev/disk4s1 1000 500 400 56% /Volumes/My Backup Drive\n";
        let volumes = parse_df(text);
        assert_eq!(volumes.len(), 2, "devfs must be filtered out");
        assert_eq!(volumes[0].mount, "/");
        assert_eq!(volumes[0].name, "Macintosh HD");
        assert_eq!(volumes[0].total_bytes, 971_350_180 * 1024);
        assert_eq!(volumes[1].mount, "/Volumes/My Backup Drive");
        assert_eq!(volumes[1].name, "My Backup Drive");
    }
}
