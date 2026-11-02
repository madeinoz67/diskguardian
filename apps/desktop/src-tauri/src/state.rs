use diskguardian_core::{Collector, Tree};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Volume-scan context (C29): present on a `Scan` iff its root was a mount point.
/// The anchor itself is re-measured at every projection (one statfs); only the
/// slow-moving decomposition components are measured once, here, at scan completion.
pub struct VolumeScan {
    /// The mount the anchor is measured against — the scan root as requested.
    pub mount: PathBuf,
    /// The parsed container report, as `diskutil` answered at scan completion. `None`
    /// when the root cannot be mapped or the parse refused: absent, never zeros.
    pub decomposition: Option<crate::volume::ContainerReport>,
    /// "Up to N": an annotation beside the breakdown, never a slice of the residual.
    pub purgeable: Option<u64>,
    /// Counted, never sized (C27).
    pub snapshot_count: u64,
    pub snapshot_note: String,
    /// Unix seconds of the decomposition measurement, for the "measured at scan time"
    /// line.
    pub measured_at: u64,
}

pub struct Scan {
    pub tree: Tree,
    pub collector: Collector,
    pub scan_root: PathBuf,
    /// `None` for a folder scan: no anchor, no boundary, no hidden node.
    pub volume: Option<VolumeScan>,
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
    /// Test seam for the volume probes (C29), the `admin_launch` pattern: when set,
    /// the anchor and the decomposition probes read through these instead of the real
    /// OS calls, so the hidden-space payload is assertable without a real volume.
    /// Production leaves it `None`.
    pub volume_probes: Mutex<Option<crate::volume::VolumeProbes>>,
}

impl AppState {
    /// Register a finished scan and return its id.
    pub fn insert(&self, tree: Tree) -> String {
        self.insert_with(tree, None)
    }

    /// As [`insert`], carrying volume-scan context when the root was a mount point.
    pub fn insert_with(&self, tree: Tree, volume: Option<VolumeScan>) -> String {
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
            volume,
        };
        self.scans
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), scan);
        id
    }
}
