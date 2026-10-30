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
