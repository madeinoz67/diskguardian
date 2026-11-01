//! Platform integration that is genuinely native, per OS.
//!
//! Everything here is a thin adapter over the shell the OS already ships: no AppKit, no
//! Win32, no GTK. On a platform with no equivalent the call degrades to opening the item
//! in the default handler rather than failing.

use std::path::Path;
use std::process::Command;

/// Reveal an item in the system file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|e| format!("could not run open: {e}"))?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("open -R exited with {status}"))
        };
    }

    #[cfg(windows)]
    {
        // explorer.exe returns 1 even when it succeeds, so the exit status proves nothing.
        Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not run explorer: {e}"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let folder = path.parent().unwrap_or(path);
        Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not run xdg-open: {e}"))
    }
}

/// Show a quick preview of an item, using whatever the platform offers.
///
/// macOS has real Quick Look. Windows and Linux have no equivalent that can be driven this
/// way, so the item opens in its default application; the caller's contract ("show me this
/// without leaving the app") is honoured as closely as the platform allows.
pub fn quicklook(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return Command::new("qlmanage")
            .arg("-p")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not run qlmanage: {e}"));
    }

    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open the file: {e}"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not run xdg-open: {e}"))
    }
}
