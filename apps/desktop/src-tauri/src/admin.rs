//! Administrator measurement: the runner and the `scan_restricted` command.
//!
//! The privileged side of this feature is a one-shot helper binary
//! (`diskguardian-admin`, `crates/admin`) driven entirely by a JSON job file. This
//! module builds the job from the stored tree, launches the helper through the OS's
//! own administrator-consent dialog, and grafts the results back into the same tree.
//!
//! The webview supplies a scan id and nothing else. Every path the helper walks is
//! derived server-side from the stored tree, so a compromised webview cannot aim root
//! at an arbitrary directory (design §5 T4) — and no disk-derived string (folder
//! names, scan paths) ever reaches the dialog source or any child argv (§5 T1): the
//! template below interpolates exactly two app-generated paths, asserted against a
//! charset before use.

use crate::state::AppState;
use diskguardian_core::{build_view, NodeId, ViewNode, ViewOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, State};

pub const HELPER_NAME: &str = "diskguardian-admin";
/// One deadline for the whole batched pass. The helper enforces it on itself (a root
/// child cannot be killed from here), and this side stops waiting a fixed grace after.
pub const DEADLINE_SECS: u64 = 900;
const OUTER_GRACE_SECS: u64 = 120;

#[derive(Debug, Clone, Serialize)]
pub struct AdminScanReport {
    /// How many restricted roots received measured subtrees.
    pub patched: u32,
    /// How many remain restricted after the pass (refused by TCC, past the deadline,
    /// or never answered). Reported, never hidden.
    pub still_restricted: u32,
    pub bytes: u64,
    pub files: u64,
    /// A fresh projection of the whole tree, the same shape every other command returns.
    pub view: Vec<ViewNode>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AdminProgress {
    pub measured: u32,
    pub total: u32,
}

/// What the launcher saw of the helper process.
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub success: bool,
    pub stderr: String,
}

/// How the helper is launched. Production on macOS is the osascript administrator
/// dialog; the IPC tests inject a stand-in so the command can be driven without a
/// password prompt. The protocol above the launcher (job file in, results file out)
/// is identical either way, which is what makes a future SMAppService launcher a
/// one-module swap.
pub type Launch = Arc<dyn Fn(&Path, &Path) -> io::Result<LaunchOutcome> + Send + Sync>;

// ── the fixed dialog template (design §3.3) ─────────────────────────────────────

/// The only string ever handed to `osascript -e`: a fixed template with exactly two
/// interpolations, both app-generated paths (the resolved helper, this pass's job
/// file). Belt and braces on top of correct AppleScript escaping: anything outside
/// `[A-Za-z0-9._/ -]` is refused outright, because the paths come from `current_exe`
/// and `$TMPDIR`, and a user path containing, say, a parenthesis should fail loudly
/// here rather than reach a shell unreviewed.
pub fn applescript_source(helper: &str, job: &str) -> Result<String, String> {
    for (what, path) in [("helper path", helper), ("job file path", job)] {
        if !template_safe(path) {
            return Err(format!(
                "refusing to build the administrator dialog: the {what} {path:?} \
                 contains characters outside [A-Za-z0-9._/ -]"
            ));
        }
    }
    Ok(format!(
        "do shell script \"{helper} --job {job}\" with administrator privileges"
    ))
}

fn template_safe(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-' | b' '))
}

/// The child argv for the dialog: the template and nothing else. Public because the
/// injection-invariance test pins it byte-for-byte.
pub fn osascript_args(source: &str) -> Vec<String> {
    vec!["-e".to_string(), source.to_string()]
}

/// The job document: version, one budget, the topmost restricted paths. Its schema is
/// closed (`deny_unknown_fields`) on the helper side.
pub fn job_document(paths: &[String], deadline_secs: u64) -> String {
    #[derive(Serialize)]
    struct JobDocument {
        version: u64,
        deadline_secs: u64,
        paths: Vec<String>,
    }
    serde_json::to_string(&JobDocument {
        version: 1,
        deadline_secs,
        paths: paths.to_vec(),
    })
    .expect("serializing the job document cannot fail")
}

#[derive(Deserialize)]
struct ResultLine {
    version: u64,
    path: String,
    status: String,
    tree: diskguardian_core::Tree,
}

/// Measure the restricted areas of a stored scan with the user's OS-mediated consent.
///
/// One prompt per pass: every topmost restricted root is batched into a single job.
/// Refused (dialog cancelled), failed or timed out, the tree is left exactly as it
/// was — the unprivileged scan is complete on its own (A5).
#[tauri::command]
pub async fn scan_restricted<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<AdminScanReport, String> {
    // 1. What needs measuring, derived from the stored tree — never from arguments.
    let expected: Vec<(NodeId, String)> = {
        let scans = lock_scans(&state)?;
        let scan = scans.get(&scan_id).ok_or("unknown scan")?;
        let roots = scan.tree.restricted_roots();
        if roots.is_empty() {
            return Err("nothing to measure: this scan has no restricted areas".to_string());
        }
        roots
            .into_iter()
            .map(|id| (id, scan.tree.path_of(id).display().to_string()))
            .collect()
    };

    // 2. The job: a fresh per-user temp directory, contents private to this user.
    //    Disk-derived names live here, as JSON, and nowhere else.
    let job_dir =
        tempfile::tempdir().map_err(|e| format!("could not create a job directory: {e}"))?;
    let job_path = job_dir.path().join("job.json");
    let paths: Vec<String> = expected.iter().map(|(_, p)| p.clone()).collect();
    std::fs::write(&job_path, job_document(&paths, DEADLINE_SECS))
        .map_err(|e| format!("could not write the job file: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&job_path, std::fs::Permissions::from_mode(0o600));
    }

    // 3. Where the helper is. The env override exists for development; an installed
    //    app resolves it inside its own bundle, which unprivileged writers cannot swap.
    let injected: Option<Launch> = state
        .admin_launch
        .lock()
        .map_err(|_| "the launcher seam is poisoned".to_string())?
        .clone();
    let helper: PathBuf = match resolve_helper() {
        Some(path) => path,
        // A test launcher does not need a real helper to exist.
        None if injected.is_some() => PathBuf::new(),
        None => {
            return Err(format!(
                "could not find the {HELPER_NAME} helper: looked in $DISKGUARDIAN_ADMIN_BIN \
                 and beside the app binary"
            ));
        }
    };

    // 4. Run it off the UI thread, exactly like `scan` does (A4).
    let launch_job = job_path.clone();
    let launch_helper = helper.clone();
    let handle = tauri::async_runtime::spawn_blocking(move || match injected {
        Some(launch) => launch(&launch_job, &launch_helper),
        None => run_with_dialog(
            &launch_helper,
            &launch_job,
            Duration::from_secs(DEADLINE_SECS + OUTER_GRACE_SECS),
        ),
    });

    // Status-line progress while the pass runs: count completed lines in the results
    // file. No petal animation in this slice.
    let done = Arc::new(AtomicBool::new(false));
    let total = expected.len() as u32;
    let progress_app = app.clone();
    let progress_job = job_path.clone();
    let progress_done = Arc::clone(&done);
    let poller = std::thread::spawn(move || {
        let mut last = u32::MAX;
        while !progress_done.load(Ordering::Relaxed) {
            let measured = count_results(&progress_job);
            if measured != last {
                last = measured;
                let _ = progress_app.emit("admin-progress", AdminProgress { measured, total });
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let measured = count_results(&progress_job);
        if measured != last {
            let _ = progress_app.emit("admin-progress", AdminProgress { measured, total });
        }
    });

    let joined = handle
        .await
        .map_err(|e| format!("the administrator pass failed to run: {e}"))?;
    done.store(true, Ordering::Relaxed);
    let _ = poller.join();

    let outcome = joined.map_err(|e| format!("could not launch the administrator helper: {e}"))?;
    if !outcome.success {
        let stderr = outcome.stderr.trim();
        // Error -128 is AppleScript's "user canceled"; the message text is localized,
        // the number is not.
        if stderr.contains("-128") || stderr.to_lowercase().contains("cancel") {
            return Err(
                "Administrator access was declined — restricted areas stay unmeasured".to_string(),
            );
        }
        return Err(format!("administrator measurement failed: {stderr}"));
    }

    // 5. Verify each result against the path it claims, then graft. The helper ran as
    //    root over a file in a directory this process owns; trusting the bytes without
    //    checking them would make that the weakest link in the tree.
    let by_path: HashMap<&str, NodeId> = expected.iter().map(|(id, p)| (p.as_str(), *id)).collect();
    let mut patched = 0u32;
    for line in read_results(&job_path) {
        let Some(&id) = by_path.get(line.path.as_str()) else {
            continue;
        };
        if line.version != 1 {
            continue;
        }
        if line.tree.scanned_root.display().to_string() != line.path {
            continue; // measured somewhere else: not graftable, not fatal
        }
        if line.status == "restricted" {
            continue; // unreadable even elevated (TCC): the node already says so
        }
        let mut scans = lock_scans(&state)?;
        let Some(scan) = scans.get_mut(&scan_id) else {
            return Err("unknown scan".to_string());
        };
        let graftable = scan
            .tree
            .nodes
            .get(id)
            .is_some_and(|node| !node.missing && id != scan.tree.root);
        if graftable && scan.tree.graft(id, &line.tree).is_ok() {
            patched += 1;
        }
    }

    // 6. The report: what cleared, what did not, and a fresh view of everything.
    let report = {
        let scans = lock_scans(&state)?;
        let scan = scans.get(&scan_id).ok_or("unknown scan")?;
        AdminScanReport {
            patched,
            still_restricted: scan.tree.restricted_roots().len() as u32,
            bytes: scan.tree.total_bytes(),
            files: scan.tree.file_count(),
            view: build_view(&scan.tree, scan.tree.root, &ViewOptions::default()),
        }
    };
    drop(job_dir); // the results are grafted; the job directory goes with the pass
    Ok(report)
}

fn lock_scans(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, std::collections::HashMap<String, crate::state::Scan>>, String>
{
    state
        .scans
        .lock()
        .map_err(|_| "scan state is poisoned by an earlier panic".to_string())
}

fn resolve_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DISKGUARDIAN_ADMIN_BIN") {
        return Some(PathBuf::from(path));
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(HELPER_NAME)))
        .filter(|path| path.exists())
}

/// Lines of the results file, skipping any that do not parse: a tampered file is an
/// integrity problem, not a reason to lose the whole pass.
fn read_results(job_path: &Path) -> Vec<ResultLine> {
    let Ok(text) =
        std::fs::read_to_string(job_path.parent().expect("job dir").join("results.jsonl"))
    else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn count_results(job_path: &Path) -> u32 {
    let Ok(text) =
        std::fs::read_to_string(job_path.parent().expect("job dir").join("results.jsonl"))
    else {
        return 0;
    };
    text.lines().filter(|line| !line.is_empty()).count() as u32
}

/// Raise the OS dialog and wait for the helper it approves. The wait is bounded: a
/// user who leaves the dialog up past the budget gets a typed timeout and an
/// untouched tree, and can simply try again.
#[cfg(target_os = "macos")]
fn run_with_dialog(helper: &Path, job: &Path, outer: Duration) -> io::Result<LaunchOutcome> {
    let source = applescript_source(&helper.display().to_string(), &job.display().to_string())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut child = std::process::Command::new("osascript")
        .args(osascript_args(&source))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("could not run osascript: {e}")))?;
    let mut stderr_pipe = child.stderr.take();

    let deadline = Instant::now() + outer;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| io::Error::new(e.kind(), format!("waiting on osascript: {e}")))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            // Killing osascript is ours to do; the elevated child, if it started, is
            // not — it finishes its job file and writes results nobody will graft.
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the administrator pass ran out of time",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let mut stderr = String::new();
    if let Some(pipe) = stderr_pipe.as_mut() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Ok(LaunchOutcome {
        success: status.success(),
        stderr,
    })
}

#[cfg(not(target_os = "macos"))]
fn run_with_dialog(_helper: &Path, _job: &Path, _outer: Duration) -> io::Result<LaunchOutcome> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "administrator measurement is macOS-only for now",
    ))
}
