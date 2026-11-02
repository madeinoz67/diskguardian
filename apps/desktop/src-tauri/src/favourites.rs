//! Favourite folders: the folders the user wants one click away on the overview.
//!
//! The list is user data that must survive relaunch, so it lives in a JSON file this
//! side owns inside the app data directory — never `localStorage`, which the OS may
//! evict and which no test without a webview can reach. The webview can only touch the
//! list through four commands whose inputs are a `scan_id`+`node_id` (the path
//! resolved here, from the tree) or a shape-validated path.
//!
//! Path identity is the canonical form — the C17 lesson. `Tree::path_of` returns the
//! literal walked path while the deletion guard canonicalises, and comparing the two
//! literally once made delete prune nothing. Favourites key on `fs::canonicalize`
//! best-effort at write, and canonicalise both sides at comparison, so `/tmp/x` and
//! `/private/tmp/x` are one entry.
//!
//! **Stated limit:** `fs::canonicalize` does not case-fold on macOS, so `/Users/NAME/x`
//! and `/Users/name/x` remain two entries on a case-insensitive volume. This can only
//! arise from typing a case variant of an existing favourite into the manual path
//! field (the scan screen returns the disk's own spelling), and the failure mode is a
//! visible duplicate row, never data loss. Per-platform case folding is deferred; the
//! reopen trigger is a real duplicate row reported from a real disk.

use crate::state::AppState;
use diskguardian_core::NodeKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{Manager, State};

/// One favourite folder, as the overview renders it.
#[derive(Debug, Clone, Serialize)]
pub struct FavouriteEntry {
    /// The store's own string; the UI passes it back verbatim to unstar.
    pub path: String,
    /// False when the folder is gone or its volume is unmounted. The row stays; A3's
    /// spirit applied to lists — reported, never silently dropped.
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FavouritesReport {
    pub favourites: Vec<FavouriteEntry>,
    /// Corrupt-store recovery, shown as visible text. A failed read is never allowed
    /// to read as "no favourites".
    pub notice: Option<String>,
}

/// Whether the viewed node and its children are favourites, resolved by path here —
/// never guessed in the UI, which has no paths.
#[derive(Debug, Clone, Serialize)]
pub struct FavouriteMarks {
    pub self_: bool,
    pub children: Vec<u64>,
}

/// The on-disk shape. `deny_unknown_fields` so a future field is a refusal (and a
/// recovery), not a silently ignored change.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreFile {
    version: u32,
    paths: Vec<String>,
}

/// The favourites store. Held as `Mutex<Favourites>` in `AppState`; one file, one
/// process. Two windows share the process and serialise on the mutex; two processes
/// would be last-write-wins, which is out of scope and stated.
#[derive(Debug, Default)]
pub struct Favourites {
    /// Resolved lazily on first command use; `None` = not yet resolved.
    path: Option<PathBuf>,
    /// Canonical-at-write keys, in insertion order.
    paths: Vec<String>,
    loaded: bool,
    /// Set when a corrupt store was recovered this load.
    notice: Option<String>,
}

impl Favourites {
    /// Test seam: point the store at a chosen file before its first use, the
    /// `admin_launch` pattern. Production resolves the app data directory on the first
    /// command instead, because `AppState::default()` runs in `configure` before an
    /// `AppHandle` exists.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    /// Resolve the store location from the app handle, once.
    fn resolve<R: tauri::Runtime>(&mut self, app: &tauri::AppHandle<R>) -> Result<(), String> {
        if self.path.is_some() {
            return Ok(());
        }
        let dir = app.path().app_data_dir().map_err(|e| {
            format!(
                "favourites are unavailable: the app data directory could not be resolved ({e})"
            )
        })?;
        self.path = Some(dir.join("favourites.json"));
        Ok(())
    }

    /// Read the store once. Malformed JSON and unknown versions take the corrupt path:
    /// the file is renamed aside, the list starts empty, and the notice says so.
    fn load(&mut self) {
        let Some(path) = self.path.clone() else {
            // No location yet — nothing has resolved it, so there is nothing to read.
            return;
        };
        self.loaded = true;
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<StoreFile>(&text) {
                Ok(store) if store.version == 1 => self.paths = store.paths,
                _ => self.recover(&path),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => self.recover(&path),
        }
    }

    /// A store that cannot be understood is preserved beside a fresh one and reported.
    fn recover(&mut self, path: &Path) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let preserved = path.with_extension(format!("json.corrupt-{stamp}"));
        self.paths.clear();
        self.notice = Some(if std::fs::rename(path, &preserved).is_ok() {
            format!(
                "the favourites list could not be read and was reset; the previous list is kept beside it as {}",
                preserved.display()
            )
        } else {
            "the favourites list could not be read and was reset".to_string()
        });
    }

    /// Serialise to a sibling `.tmp` file, then rename over the target. A torn write
    /// must never lose the list — unlike the legend width, this is user data.
    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("favourites are unavailable: no store location".to_string());
        };
        let doc = serde_json::to_string(&StoreFile {
            version: 1,
            paths: self.paths.clone(),
        })
        .map_err(|e| format!("favourites could not be serialised: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("favourites store directory could not be created: {e}"))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, doc).map_err(|e| format!("favourites could not be written: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("favourites could not be saved: {e}"))?;
        Ok(())
    }

    /// Keying happens here. The duplicate check canonicalises both sides, so a literal
    /// stored while a folder was missing still collapses with its real spelling once
    /// the folder exists; the case-variant limit is stated in the module docs.
    fn contains(&self, key: &str) -> bool {
        self.paths.iter().any(|stored| canonical(stored) == key)
    }

    /// Add (canonical, no-op on a duplicate) or remove (exact stored string). Persists
    /// before returning; on a write failure the error names it, and the in-memory list
    /// keeps the change so the next successful save is not silently lost.
    fn set(&mut self, path: String, add: bool) -> Result<Vec<String>, String> {
        if add {
            let key = canonical(&path);
            if !self.contains(&key) {
                self.paths.push(key);
            }
        } else {
            // The UI passes the store's own string back verbatim.
            self.paths.retain(|stored| *stored != path);
        }
        self.persist()?;
        Ok(self.paths.clone())
    }
}

/// The store's canonical form of a path, best-effort: the literal string when the
/// folder has vanished between the click and the write (C25's lesson — readability
/// belongs to the stat, never to validation).
fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Shape only, the exact rule C25 taught: absolute, no `..` component. Whether the
/// folder exists or can be read is the existence stat's and the walk's business.
fn shape_is_valid(path: &str) -> bool {
    Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|component| component == std::path::Component::ParentDir)
}

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Favourites>, String> {
    state
        .favourites
        .lock()
        .map_err(|_| "favourites state is poisoned by an earlier panic".to_string())
}

/// Every favourite with an existence stat, plus the recovery notice. The stat follows
/// symlinks (a favourite stored through `/tmp` answers for `/private/tmp`) and succeeds
/// on TCC-protected directories — scanning those yields restricted petals, which is
/// correct, not "missing".
#[tauri::command]
pub fn favourites<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<FavouritesReport, String> {
    let mut store = lock(&state)?;
    store.resolve(&app)?;
    store.load();
    Ok(FavouritesReport {
        favourites: store
            .paths
            .iter()
            .map(|path| FavouriteEntry {
                path: path.clone(),
                exists: Path::new(path).exists(),
            })
            .collect(),
        notice: store.notice.clone(),
    })
}

/// Which of a node's tree children are favourites, and whether the node itself is one.
/// Read-only over the scan tree plus the already-loaded store — the same cheapness
/// `revalidate` relies on. Children the legend prunes away are harmless: the frontend
/// only consults marks for rows it draws.
#[tauri::command]
pub fn favourite_marks<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
) -> Result<FavouriteMarks, String> {
    // The scan tree is read and released first, so the two locks are never held together.
    let scans = crate::commands::lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    let node = node_id.map_or(scan.tree.root, |id| id as usize);
    if node >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    let self_path = scan.tree.path_of(node).display().to_string();
    let child_paths: Vec<(u64, String)> = scan
        .tree
        .node(node)
        .children
        .iter()
        .map(|&child| (child as u64, scan.tree.path_of(child).display().to_string()))
        .collect();
    drop(scans);

    let mut store = lock(&state)?;
    store.resolve(&app)?;
    store.load();
    Ok(FavouriteMarks {
        self_: store.contains(&canonical(&self_path)),
        children: child_paths
            .iter()
            .filter(|(_, path)| store.contains(&canonical(path)))
            .map(|(id, _)| *id)
            .collect(),
    })
}

/// Star or unstar a node of a scan. `node_id: None` maps to the tree root, so the
/// legend heading can star the scan root itself. Refuses files and nodes flagged
/// missing, with their own words.
#[tauri::command]
pub fn favourite_node<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
    add: bool,
) -> Result<Vec<String>, String> {
    let scans = crate::commands::lock_scans(&state)?;
    let scan = scans.get(&scan_id).ok_or("unknown scan")?;
    let node = node_id.map_or(scan.tree.root, |id| id as usize);
    if node >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    let target = scan.tree.node(node);
    if target.kind == NodeKind::File {
        return Err("only folders can be favourites".to_string());
    }
    if target.missing {
        return Err("that folder is no longer on disk".to_string());
    }
    // `path_of` returns the literal walked path; the store canonicalises at write.
    let path = scan.tree.path_of(node).display().to_string();
    drop(scans);

    let mut store = lock(&state)?;
    store.resolve(&app)?;
    store.load();
    store.set(path, add)
}

/// Add or remove a favourite by path (the overview's unstar, and any future caller
/// that names a folder directly). Shape-validated only.
#[tauri::command]
pub fn set_favourite<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    path: String,
    add: bool,
) -> Result<Vec<String>, String> {
    if !shape_is_valid(&path) {
        return Err("favourites need an absolute path without '..'".to_string());
    }
    let mut store = lock(&state)?;
    store.resolve(&app)?;
    store.load();
    store.set(path, add)
}
