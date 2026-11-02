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
//! best-effort at write, canonicalise both sides at comparison, canonicalise the
//! removal key too (the legend's unstar passes the walked spelling, not the store's),
//! and reconcile stored spellings when the store loads — so `/tmp/x` and
//! `/private/tmp/x` are one entry at every door, including one that was stored as a
//! literal while its folder was gone and has since come back.
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
    /// Why the list cannot be trusted right now — a corrupt store that was recovered,
    /// or a store that could not be read and was left untouched. Shown as visible
    /// text: a failed read is never allowed to read as "no favourites".
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
    /// Whether a healthy read of the store file has happened this process. One read is
    /// the authority: re-reading on every command would discard any change a failed
    /// write left in the list, which is exactly what the retention in `set` promises
    /// not to do. An unreadable store leaves this `false`, so the next command retries.
    loaded: bool,
    /// Set when a corrupt store was recovered this load.
    notice: Option<String>,
    /// Set when the last read could not read the store at all (permissions, I/O): the
    /// file is left untouched, mutations refuse until a read succeeds, and every
    /// command retries. A blip is not corruption — the list still exists on disk.
    unreadable: Option<String>,
}

impl Favourites {
    /// Test seam: point the store at a chosen file before its first use, the
    /// `admin_launch` pattern. Production resolves the app data directory on the first
    /// command instead, because `AppState::default()` runs in `configure` before an
    /// `AppHandle` exists. Everything an earlier file established is forgotten: the
    /// new file has never been read by this process.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.paths.clear();
        self.loaded = false;
        self.notice = None;
        self.unreadable = None;
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

    /// Read the store. Four outcomes, four doctrines:
    ///
    /// - A healthy read installs the list, **reconciled**: every stored spelling is
    ///   re-keyed to its canonical form where the folder exists again, and duplicates
    ///   collapse. Write-time keying cannot repair a literal that was stored while its
    ///   folder was gone, so load is where two spellings of one restored folder become
    ///   one row.
    /// - `NotFound` is a fresh machine's first read: an empty store, no drama.
    /// - Malformed JSON or an unknown version is corruption: the file is renamed aside,
    ///   the list starts empty, the notice says so (C27 — a failed read is never
    ///   allowed to read as "no favourites").
    /// - Any other read failure (permissions, I/O) is *not* corruption: the file is
    ///   left untouched, `unreadable` is set so mutations refuse, and the next command
    ///   retries. Renaming a good file aside because of a permission blip once
    ///   evicted an intact list permanently; it does not again.
    ///
    /// A healthy read happens once per process: afterwards the in-memory list is the
    /// authority, and re-reading would discard any change a failed write left there.
    fn load(&mut self) {
        let Some(path) = self.path.clone() else {
            // No location yet — nothing has resolved it, so there is nothing to read.
            return;
        };
        if self.loaded {
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<StoreFile>(&text) {
                Ok(store) if store.version == 1 => {
                    self.paths = reconcile(store.paths);
                    self.notice = None;
                    self.unreadable = None;
                    self.loaded = true;
                }
                _ => {
                    self.recover(&path);
                    self.loaded = true;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.paths.clear();
                self.unreadable = None;
                self.loaded = true;
            }
            Err(error) => {
                self.paths.clear();
                self.notice = None;
                self.unreadable = Some(format!(
                    "the favourites list could not be read ({error}); it was left untouched"
                ));
            }
        }
    }

    /// Whether nothing is starred — the answer `favourite_marks` can give without
    /// touching the scan tree at all.
    fn is_empty(&self) -> bool {
        self.paths.is_empty()
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

    /// Add (canonical, no-op on a duplicate) or remove. Both directions meet the store
    /// on canonical ground: the overview passes the store's own string back, but the
    /// legend's unstar passes `path_of`'s literal walked spelling, and a row keyed
    /// `/private/var/…` must still leave to an unstar spelled `/var/…` (the C17
    /// mismatch; comparing the literal once made that unstar a silent no-op). Persists
    /// before returning; on a write failure the error names it, and the in-memory list
    /// keeps the change — `load` reads a healthy store once per process, so the next
    /// successful save carries it rather than silently losing it.
    fn set(&mut self, path: String, add: bool) -> Result<Vec<String>, String> {
        if let Some(reason) = &self.unreadable {
            // Writing now would replace a file this process has never managed to read —
            // the eviction the read path refuses to do, done by the write path instead.
            return Err(format!(
                "favourites could not be changed while the store cannot be read ({reason})"
            ));
        }
        let key = canonical(&path);
        if add {
            if !self.contains(&key) {
                self.paths.push(key);
            }
        } else {
            self.paths.retain(|stored| canonical(stored) != key);
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

/// Reconcile stored spellings: re-key each to its canonical form where the folder
/// exists again, then keep the first of each. A literal stored while its folder was
/// gone and an older canonical spelling of the same folder are then one row, which is
/// the C28 promise; two different missing folders stay two rows, because while they
/// are gone nothing can prove them the same.
fn reconcile(paths: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .map(|path| canonical(&path))
        .filter(|path| seen.insert(path.clone()))
        .collect()
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

/// Map `scan_id` plus an optional node id to the scan and a node index, with the
/// commands' typed errors. Pure lookup — no paths are built, which is what lets a
/// command validate first and do its expensive work only when there is any to do.
fn resolve_node<'a>(
    scans: &'a std::collections::HashMap<String, crate::state::Scan>,
    scan_id: &str,
    node_id: Option<u64>,
) -> Result<(&'a crate::state::Scan, usize), String> {
    let scan = scans
        .get(scan_id)
        .ok_or_else(|| "unknown scan".to_string())?;
    let node = node_id.map_or(scan.tree.root, |id| id as usize);
    if node >= scan.tree.nodes.len() {
        return Err("node is not part of this scan".to_string());
    }
    Ok((scan, node))
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
        notice: store.notice.clone().or_else(|| store.unreadable.clone()),
    })
}

/// Which of the children the view draws are favourites, and whether the node itself is
/// one. This runs on every screen change, so it is bounded by what the screen can
/// show, never by the tree: an empty store (the common case) answers without touching
/// the tree at all, and otherwise marks are computed over `build_view`'s children —
/// at most `max_children` rows — never the full child set. Canonicalising every tree
/// child of a 100k-entry folder measured in seconds here, on a synchronous command.
#[tauri::command]
pub fn favourite_marks<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    scan_id: String,
    node_id: Option<u64>,
) -> Result<FavouriteMarks, String> {
    // Validate first, by pure lookup and with no paths built, so the typed errors for
    // unknown scans and foreign nodes keep their shape on every path below.
    {
        let scans = crate::commands::lock_scans(&state)?;
        resolve_node(&scans, &scan_id, node_id)?;
    }

    // The two mutexes are never held together: the favourites lock is taken and
    // released, then the scans lock, then the favourites lock again.
    let empty = {
        let mut store = lock(&state)?;
        store.resolve(&app)?;
        store.load();
        store.is_empty()
    };
    if empty {
        // Nothing is starred anywhere: no mark can be true, and canonicalising the tree
        // to prove it is exactly the cost this exists to skip.
        return Ok(FavouriteMarks {
            self_: false,
            children: Vec::new(),
        });
    }

    // Paths for the node itself and the children the view draws — the same pruning the
    // legend renders, so a folder with thousands of children costs a handful of
    // `path_of` calls, and a child merged into the "smaller objects" petal (no row, no
    // star to draw) is never canonicalised at all.
    let (self_path, child_paths): (String, Vec<(u64, String)>) = {
        let scans = crate::commands::lock_scans(&state)?;
        let (scan, node) = resolve_node(&scans, &scan_id, node_id)?;
        let view = diskguardian_core::build_view(
            &scan.tree,
            node,
            &diskguardian_core::ViewOptions::default(),
        );
        let drawn: Vec<u64> = view
            .first()
            .map(|root| {
                root.children
                    .iter()
                    .filter_map(|&index| view[index as usize].node_id)
                    .collect()
            })
            .unwrap_or_default();
        (
            scan.tree.path_of(node).display().to_string(),
            drawn
                .into_iter()
                .map(|id| (id, scan.tree.path_of(id as usize).display().to_string()))
                .collect(),
        )
    };

    let store = lock(&state)?;
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
    let (scan, node) = resolve_node(&scans, &scan_id, node_id)?;
    let target = scan.tree.node(node);
    if target.kind == NodeKind::File {
        return Err("only folders can be favourites".to_string());
    }
    if target.missing {
        return Err("that folder is no longer on disk".to_string());
    }
    // `path_of` returns the literal walked path; the store canonicalises at write and,
    // since the unstar fix, at removal too.
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
