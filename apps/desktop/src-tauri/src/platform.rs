//! Platform integration that is genuinely native: Finder reveal and Quick Look.
//!
//! Both shell out to the system binaries rather than reimplementing AppKit, which
//! keeps this file honest about what it is: a thin adapter, not a native layer.

use std::path::Path;
use std::process::Command;

/// Reveal an item in Finder.
pub fn reveal(path: &Path) -> Result<(), String> {
    let status = Command::new("open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(|e| format!("could not run open: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open -R exited with {status}"))
    }
}

/// Open the system Quick Look preview. Spawned, not awaited: the panel stays open
/// until the user closes it.
pub fn quicklook(path: &Path) -> Result<(), String> {
    Command::new("qlmanage")
        .arg("-p")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not run qlmanage: {e}"))
}
