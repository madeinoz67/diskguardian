//! win-elevate — the one Win32 call behind Windows self-elevation, in a cage.
//!
//! The entire `unsafe` surface of Windows elevation in this repository is the single
//! `ShellExecuteW` call below: a few dozen lines with no tauri and no core, so
//! `crates/admin` keeps `unsafe_code = "forbid"` and the app crate gains no `unsafe`
//! of its own (multi-platform release design §3.7.2). This crate's whole source is
//! the review surface for the only `unsafe` the multi-platform release adds.
//!
//! The semantic the caller needs is **prompt-then-return**: raise the UAC consent, and
//! on approval start the elevated process and return — without holding a child handle.
//! That is why the `runas` crate does not fit: its `status()` waits for the child to
//! exit, and the child is a session-long server. No handle is retained here on purpose:
//! the elevated instance's lifetime is bounded by the session protocol's own three
//! exits (stale heartbeat, staging directory gone, idle budget), because Windows
//! imposes no timeout on a UAC-elevated process — those budgets are ours, not
//! Windows'.
//!
//! Off Windows this crate compiles to nothing.
#![cfg(windows)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

/// What the elevation attempt saw. Mirrors the app's `LaunchOutcome` so the launcher
/// seam maps it one-for-one: `success` means "the elevated process started", never
/// "it finished" — it does not finish before the session does.
#[derive(Debug, Clone)]
pub struct ElevateOutcome {
    pub success: bool,
    pub stderr: String,
}

/// `ShellExecuteW` reports failure as one of the classic `SE_*` small integers
/// (result <= 32), never a valid handle. 5 is the runas decline: the user clicked No
/// or dismissed the prompt.
const SE_ERR_ACCESSDENIED: isize = 5;

/// Raise the UAC consent and start `exe` elevated with `parameters` as its command
/// line, hidden. Returns once the user has answered and, on approval, the process has
/// been started. The caller learns nothing about the child — no pid, no handle.
pub fn elevate_serve(exe: &Path, parameters: &str) -> io::Result<ElevateOutcome> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain([0]).collect();
    let file: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
    let params: Vec<u16> = OsStr::new(parameters).encode_wide().chain([0]).collect();

    // SAFETY: the three non-null pointer arguments point at NUL-terminated UTF-16
    // buffers owned by this frame and read synchronously by the call; the nulls are
    // the documented "no owner window" and "inherit the working directory" values. No
    // handle or pid is retained from the result, and nothing outlives this frame.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_HIDE,
        )
    };

    let code = result as isize;
    if code > 32 {
        return Ok(ElevateOutcome {
            success: true,
            stderr: String::new(),
        });
    }
    let stderr = if code == SE_ERR_ACCESSDENIED {
        // Worded to land in the app's existing refusal mapping ("cancel" is its
        // discriminator), so a Windows decline reads exactly like a macOS one.
        "the user cancelled the elevation prompt".to_string()
    } else {
        format!("Windows refused to start the elevated process (ShellExecuteW result {code})")
    };
    Ok(ElevateOutcome {
        success: false,
        stderr,
    })
}
