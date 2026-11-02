//! APFS local snapshots: what Time Machine is holding on this Mac.
//!
//! Local snapshots are invisible in Finder and are charged to the volume, so a user who
//! cannot see them cannot reclaim the space. This module lists them and offers to delete
//! them — and is careful about three things:
//!
//! 1. **No size is invented.** macOS reports which snapshots exist, not how much each
//!    occupies (they share blocks with the live filesystem and with each other, so a
//!    per-snapshot number would be a fiction). The list shows timestamps only, and the one
//!    honest figure — how much free space actually moved — is measured before and after a
//!    purge rather than predicted.
//! 2. **Deletion runs as the user, never elevated.** Snapshot purging needs administrator
//!    rights in practice, and the project's rule is that measurement is the only thing that
//!    runs elevated with no delete call sites anywhere in the privileged path. So this
//!    command runs `tmutil` as the logged-in user; if macOS refuses, the refusal is
//!    reported in words that name the cause, not swallowed into "0 freed".
//! 3. **Nothing disk-derived reaches a shell.** There is no shell: `tmutil` is executed
//!    with an argument vector, and the only path in that vector is the boot volume, fixed
//!    here. The webview supplies a boolean and nothing else.

use serde::Serialize;
use std::process::Command;

/// The one volume this module speaks for. Fixed here rather than accepted from the
/// caller, so no UI-chosen path can become an argument to a deleting command.
pub const BOOT_VOLUME: &str = "/";

#[cfg(target_os = "macos")]
const TMUTIL: &str = "/usr/bin/tmutil";

/// One local snapshot, named the way `tmutil` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalSnapshot {
    /// `com.apple.TimeMachine.2026-11-01-085943.local`
    pub name: String,
    /// `2026-11-01-085943` — the stamp `tmutil deletelocalsnapshots` accepts.
    pub stamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotReport {
    /// False where the platform has no local Time Machine snapshots to speak of.
    pub supported: bool,
    pub mount: String,
    pub snapshots: Vec<LocalSnapshot>,
    /// Why the listing failed, when it did. A failed listing is never shown as "none".
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PurgeReport {
    /// `tmutil` reported success.
    pub purged: bool,
    /// The OS refused for want of administrator rights. Distinct from a generic failure,
    /// because the answer for the user is different.
    pub refused: bool,
    /// What to tell the user, in full sentences, whatever happened.
    pub message: String,
    /// Measured free-space change across the attempt. Signed: another process can take
    /// space in the same window, and pretending that never happens would be a lie of
    /// arithmetic rather than of intent.
    pub freed_bytes: i64,
    /// The snapshots that are still there afterwards.
    pub remaining: Vec<LocalSnapshot>,
}

/// What a `tmutil` invocation produced, reduced to what this module needs. A seam rather
/// than `std::process::Output` so a test can stand in for the real command without
/// building one.
#[derive(Debug, Clone)]
pub struct TmOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub type Runner = fn(&[String]) -> TmOutput;

static RUNNER: std::sync::Mutex<Option<Runner>> = std::sync::Mutex::new(None);

/// Replace the `tmutil` runner. Tests only: the real path is the default.
pub fn set_runner_for_tests(runner: Option<Runner>) {
    *RUNNER.lock().expect("tmutil runner slot") = runner;
}

fn run_tmutil(args: &[String]) -> TmOutput {
    let injected = *RUNNER.lock().expect("tmutil runner slot");
    if let Some(run) = injected {
        return run(args);
    }

    #[cfg(target_os = "macos")]
    {
        match Command::new(TMUTIL).args(args).output() {
            Ok(output) => TmOutput {
                ok: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Err(error) => TmOutput {
                ok: false,
                stdout: String::new(),
                stderr: format!("could not run {TMUTIL}: {error}"),
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = args;
        TmOutput {
            ok: false,
            stdout: String::new(),
            stderr: "local Time Machine snapshots exist only on macOS".to_string(),
        }
    }
}

/// `YYYY-MM-DD-HHMMSS`, the only shape `tmutil` accepts as a date.
fn is_stamp(stamp: &str) -> bool {
    let parts: Vec<&str> = stamp.split('-').collect();
    match parts.as_slice() {
        [year, month, day, time] => {
            year.len() == 4
                && month.len() == 2
                && day.len() == 2
                && time.len() == 6
                && parts
                    .iter()
                    .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        }
        _ => false,
    }
}

/// Pull the snapshots out of `tmutil listlocalsnapshots` output.
///
/// Everything that is not a well-formed snapshot line is ignored: the header
/// (`Snapshots for disk /:`), the empty "no snapshots" answer, and anything a future
/// macOS might add. A line only counts when it is the whole name, so a path or a message
/// that happens to contain a snapshot name is not mistaken for one.
pub fn parse_listing(stdout: &str) -> Vec<LocalSnapshot> {
    stdout
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            let stamp = line
                .strip_prefix("com.apple.TimeMachine.")?
                .strip_suffix(".local")?;
            if !is_stamp(stamp) {
                return None;
            }
            Some(LocalSnapshot {
                name: line.to_string(),
                stamp: stamp.to_string(),
            })
        })
        .collect()
}

/// Free space on the boot volume, measured now.
fn free_bytes() -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|disk| disk.mount_point() == std::path::Path::new(BOOT_VOLUME))
        .map(sysinfo::Disk::available_space)
        .next()
}

pub fn list_snapshots() -> SnapshotReport {
    if !cfg!(target_os = "macos") {
        return SnapshotReport {
            supported: false,
            mount: BOOT_VOLUME.to_string(),
            snapshots: Vec::new(),
            error: None,
        };
    }

    let output = run_tmutil(&["listlocalsnapshots".to_string(), BOOT_VOLUME.to_string()]);
    // `tmutil` exits 0 for "no snapshots", so a non-zero exit is a real failure and must
    // not be rendered as an empty list.
    let error = if output.ok {
        None
    } else {
        Some(output.stderr.trim().to_string())
    };

    SnapshotReport {
        supported: true,
        mount: BOOT_VOLUME.to_string(),
        snapshots: parse_listing(&output.stdout),
        error,
    }
}

/// Delete every local snapshot on the boot volume, as the user.
///
/// Never elevated: see the module docs. A refusal is a result with a message, not an error
/// that leaves the UI guessing.
pub fn purge_snapshots(confirmed: bool) -> Result<PurgeReport, String> {
    if !confirmed {
        return Err("purging local snapshots needs explicit confirmation".to_string());
    }
    if !cfg!(target_os = "macos") {
        return Err("local Time Machine snapshots exist only on macOS".to_string());
    }

    let before = free_bytes();
    let output = run_tmutil(&["deletelocalsnapshots".to_string(), BOOT_VOLUME.to_string()]);
    let after = free_bytes();

    let freed_bytes = match (before, after) {
        (Some(before), Some(after)) => after as i64 - before as i64,
        _ => 0,
    };

    let remaining = list_snapshots().snapshots;
    let stderr = output.stderr.trim().to_lowercase();
    let refused = !output.ok
        && (stderr.contains("root privilege")
            || stderr.contains("not permitted")
            || stderr.contains("operation not permitted"));

    let message = if output.ok {
        format!(
            "Deleted the local snapshots. macOS reported the space back to the volume: {:.1} GB.",
            freed_bytes as f64 / 1_073_741_824.0
        )
    } else if refused {
        "macOS refused: deleting local snapshots needs administrator rights, and this app \
         deliberately deletes nothing with them. Delete them in System Settings → General → \
         Time Machine, or run `sudo tmutil deletelocalsnapshots /` yourself."
            .to_string()
    } else {
        format!(
            "tmutil could not delete the snapshots: {}",
            output.stderr.trim()
        )
    };

    Ok(PurgeReport {
        purged: output.ok,
        refused,
        message,
        freed_bytes,
        remaining,
    })
}

/// What Time Machine is holding on the boot volume.
///
/// `async` because `tmutil` answers in tens of milliseconds normally but can block on a
/// busy volume, and a sync command would block the UI thread while it does.
#[tauri::command(async)]
pub fn local_snapshots() -> SnapshotReport {
    list_snapshots()
}

/// Delete every local snapshot on the boot volume, as the user. Always confirms first.
#[tauri::command(async)]
pub fn purge_local_snapshots(confirmed: bool) -> Result<PurgeReport, String> {
    purge_snapshots(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_listing_parses_into_named_snapshots() {
        let listing = "Snapshots for disk /:\n\
                       com.apple.TimeMachine.2026-11-01-085943.local\n\
                       com.apple.TimeMachine.2026-11-02-080842.local\n";
        let found = parse_listing(listing);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].stamp, "2026-11-01-085943");
        assert_eq!(
            found[1].name,
            "com.apple.TimeMachine.2026-11-02-080842.local"
        );
    }

    #[test]
    fn the_header_and_the_empty_answer_are_not_snapshots() {
        assert!(parse_listing("Snapshots for disk /:\n").is_empty());
        assert!(parse_listing("No snapshots for disk /.\n").is_empty());
        assert!(parse_listing("").is_empty());
    }

    #[test]
    fn a_malformed_stamp_is_ignored_rather_than_passed_on() {
        // Every one of these would be a bad argument to `tmutil deletelocalsnapshots` if
        // it were ever fed back, and none of them is a snapshot this app should name.
        for bad in [
            "com.apple.TimeMachine..local",
            "com.apple.TimeMachine.2026-11-01.local",
            "com.apple.TimeMachine.2026-11-01-08594.local",
            "com.apple.TimeMachine.2026-11-01-0859430.local",
            "com.apple.TimeMachine.26-11-01-085943.local",
            "com.apple.TimeMachine.2026-11-01-08x943.local",
            "com.apple.TimeMachine.com.apple.TimeMachine.2026-11-01-085943.local",
        ] {
            assert!(
                parse_listing(bad).is_empty(),
                "{bad} must not parse as a snapshot"
            );
        }
    }

    #[test]
    fn a_snapshot_name_inside_a_sentence_is_not_a_snapshot() {
        let noisy = "warning: com.apple.TimeMachine.2026-11-01-085943.local could not be read\n";
        assert!(parse_listing(noisy).is_empty());
    }

    #[test]
    fn stamps_are_the_only_accepted_date_shape() {
        assert!(is_stamp("2026-11-01-085943"));
        assert!(!is_stamp("2026-11-01"));
        assert!(!is_stamp("2026-11-01-085943-extra"));
        assert!(!is_stamp(""));
    }
}
