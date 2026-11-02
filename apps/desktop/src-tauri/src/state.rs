use diskguardian_core::{Collector, Tree};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Scan {
    pub tree: Tree,
    pub collector: Collector,
    pub scan_root: PathBuf,
}

#[derive(Default)]
pub struct AppState {
    pub scans: Mutex<HashMap<String, Scan>>,
    pub next_scan: Mutex<u64>,
    /// The administrator measurement session: one staging directory and one spent
    /// dialog per app run (principal's 2026-11-02 amendment). Dropped with the app,
    /// which removes the staging directory and ends the privileged helper.
    /// Async because the one launch that may start it is user-paced (the OS dialog)
    /// and concurrent passes must queue behind it, not stack a second prompt.
    pub admin_session: tauri::async_runtime::Mutex<Option<crate::admin::AdminSession>>,
    /// Test seam for administrator measurement: when set, `scan_restricted` starts the
    /// serving helper with this instead of raising the real osascript dialog.
    /// Production leaves it `None`; the job/results protocol is identical either way.
    pub admin_launch: Mutex<Option<crate::admin::Launch>>,
    /// The flag the walk in flight is watching, if there is one. One cell, written by whoever
    /// registers a scan most recently and read only by `cancel_scan` — so the operation this
    /// can express is "cancel the most recently registered walk", not "cancel the walk I
    /// superseded". Correcting that needs a per-request identity (ISA R14).
    pub scan_cancel: Mutex<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
    /// The favourite folders: a Rust-owned JSON store, resolved to the app data
    /// directory lazily on first command use (`default()` runs before an `AppHandle`
    /// exists). The webview reaches it only through the four favourites commands.
    pub favourites: Mutex<crate::favourites::Favourites>,
}

impl AppState {
    /// Register a finished scan and return its id.
    pub fn insert(&self, tree: Tree) -> String {
        let mut counter = self
            .next_scan
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *counter += 1;
        let id = format!("scan-{counter}");
        let scan_root = tree.scanned_root.clone();
        let scan = Scan {
            tree,
            collector: Collector::new(),
            scan_root,
        };
        self.scans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), scan);
        id
    }
}
