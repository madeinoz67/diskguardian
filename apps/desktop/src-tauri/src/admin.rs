//! Administrator measurement: the session runner and the `scan_restricted` command.
//!
//! The privileged side of this feature is the `diskguardian-admin` helper
//! (`crates/admin`). Since the principal's 2026-11-02 amendment it is one process per
//! app session: the first pass raises the OS's administrator-consent dialog once and
//! starts the helper in `--serve` mode; later passes hand it job files and get results
//! back, with no second prompt. The job/results protocol is unchanged, so a future
//! SMAppService launcher replaces this module's launcher and nothing else.
//!
//! The webview supplies a scan id and nothing else. Every path the helper walks is
//! derived server-side from the stored tree, so a compromised webview cannot aim root
//! at an arbitrary directory (design §5 T4) — and no disk-derived string (folder
//! names, scan paths) ever reaches the dialog source or any child argv (§5 T1): the
//! template below interpolates exactly two app-generated paths, asserted against a
//! charset before use.
//!
//! The session ends with the app: the staging directory is a `TempDir` owned by
//! [`AdminSession`], whose drop removes it, and the helper exits when that directory
//! disappears (or its heartbeat goes stale, or it goes idle — see `crates/admin`).
//! There is no privileged process between app runs.

use crate::state::AppState;
use diskguardian_core::{NodeId, ViewNode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
// `Read` is a trait bound for the helper's stdout reads, which only exist on macOS.
#[cfg(target_os = "macos")]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, State};

pub const HELPER_NAME: &str = "diskguardian-admin";
/// One deadline for one batched pass. The helper enforces it on itself (a root child
/// cannot be killed from here), and this side stops waiting a fixed grace after.
pub const DEADLINE_SECS: u64 = 900;
const OUTER_GRACE_SECS: u64 = 120;
/// After the dialog is approved, how long to wait for the helper to announce itself
/// before deciding it never started.
const READY_WAIT: Duration = Duration::from_secs(30);
/// How often the session heartbeat is touched. The helper's staleness budget (30 s)
/// is six beats wide, so one lost beat cannot end a live session.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct AdminScanReport {
    /// Which scan this was grafted into. A privileged pass can outlive the scan that asked
    /// for it — the user picks another folder while root is still walking — so the report
    /// has to name its own subject or the frontend cannot tell whether to paint it.
    pub scan_id: String,
    /// How many restricted roots received measured subtrees.
    pub patched: u32,
    /// How many remain restricted after the pass (refused by TCC, past the deadline,
    /// or never answered). Reported, never hidden.
    pub still_restricted: u32,
    pub bytes: u64,
    pub files: u64,
    /// A fresh projection of the whole tree, the same shape every other command returns.
    pub view: Vec<ViewNode>,
    /// Volume-level figures when the subject is a volume scan: a graft grows the tree,
    /// so the hidden residual shrinks in the same projection.
    pub volume: Option<crate::commands::VolumeSummary>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AdminProgress {
    pub measured: u32,
    pub total: u32,
}

/// What the launcher saw.
#[derive(Debug, Clone)]
pub struct LaunchOutcome {
    pub success: bool,
    pub stderr: String,
}

/// How the serving helper is started, given the session's staging directory.
/// Production on macOS is the osascript administrator dialog; the IPC tests inject a
/// stand-in that plays the daemon against the same files, so the command can be driven
/// without a password prompt. The protocol above the launcher (job files in, results
/// files out) is identical either way, which is what makes a future SMAppService
/// launcher a one-module swap.
pub type Launch = Arc<dyn Fn(&Path) -> io::Result<LaunchOutcome> + Send + Sync>;

/// One privileged session: a staging directory that lives as long as the app does,
/// a heartbeat the helper watches, and whether the one dialog has been spent.
pub struct AdminSession {
    dir: tempfile::TempDir,
    daemon_started: bool,
    job_seq: u64,
    stop_heartbeat: Arc<AtomicBool>,
    heartbeat: Option<std::thread::JoinHandle<()>>,
}

impl AdminSession {
    fn new() -> io::Result<Self> {
        let dir = tempfile::tempdir()?;
        // The heartbeat exists before the daemon can, so a freshly started helper
        // never sees "no heartbeat ever" as staleness.
        touch(&dir.path().join("session.alive"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                dir.path().join("session.alive"),
                std::fs::Permissions::from_mode(0o600),
            );
        }
        let stop_heartbeat = Arc::new(AtomicBool::new(false));
        let beat_dir = dir.path().to_path_buf();
        let beat_stop = Arc::clone(&stop_heartbeat);
        let heartbeat = std::thread::spawn(move || {
            while !beat_stop.load(Ordering::Relaxed) && beat_dir.is_dir() {
                let _ = std::fs::write(beat_dir.join("session.alive"), b"1");
                std::thread::sleep(HEARTBEAT_EVERY);
            }
        });
        Ok(Self {
            dir,
            daemon_started: false,
            job_seq: 0,
            stop_heartbeat,
            heartbeat: Some(heartbeat),
        })
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Drop for AdminSession {
    fn drop(&mut self) {
        self.stop_heartbeat.store(true, Ordering::Relaxed);
        if let Some(handle) = self.heartbeat.take() {
            let _ = handle.join();
        }
        // `dir` drops last and removes the staging directory: the helper's clean-exit
        // signal. A crashed app leaves the directory behind, and the helper's
        // staleness and idle budgets end it there.
    }
}

fn touch(path: &Path) -> io::Result<()> {
    std::fs::write(path, b"1")
}

// ── the fixed dialog template (design §3.3, amended) ────────────────────────────

/// The only string ever handed to `osascript -e`: a fixed template with exactly two
/// interpolations, both app-generated paths (the resolved helper, this session's
/// staging directory). The trailing redirect-and-background is what makes the dialog
/// return while the helper keeps serving — and it is fixed text, not interpolated.
///
/// Belt and braces on top of correct AppleScript escaping: anything outside
/// `[A-Za-z0-9._/ -]` is refused outright, because both paths come from `current_exe`
/// and `$TMPDIR`, and a user path containing, say, a parenthesis should fail loudly
/// here rather than reach a shell unreviewed.
pub fn applescript_source(helper: &str, serve_dir: &str) -> Result<String, String> {
    for (what, path) in [("helper path", helper), ("staging directory", serve_dir)] {
        if !template_safe(path) {
            return Err(format!(
                "refusing to build the administrator dialog: the {what} {path:?} \
                 contains characters outside [A-Za-z0-9._/ -]"
            ));
        }
    }
    Ok(format!(
        "do shell script \"{helper} --serve {serve_dir} > /dev/null 2>&1 &\" \
         with administrator privileges"
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

#[derive(Deserialize)]
struct JobStatus {
    exit: u8,
    #[serde(default)]
    reason: Option<String>,
}

/// Measure the restricted areas of a stored scan with the user's OS-mediated consent.
///
/// Whether this app run already holds an authorised helper.
///
/// The cadence is one prompt per session, so after the first approval a later scan should
/// measure without asking again — and without re-offering a button that implies it will.
#[tauri::command]
pub async fn admin_authorised<R: tauri::Runtime>(
    // Bound so the runtime type is inferable; an injected handle, never a message argument.
    _app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let guard = state.admin_session.lock().await;
    Ok(guard.as_ref().is_some_and(|session| session.daemon_started))
}

/// One prompt per app session: the first pass starts the serving helper behind the OS
/// dialog, and later passes reuse it. Refused (dialog cancelled), failed or timed out,
/// the tree is left exactly as it was — the unprivileged scan is complete on its own
/// (A5), and a refusal only means the next pass asks again.
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

    // 2 + 3. The session, its job file, and the one dialog. All under the session
    //    lock, held across the launch on purpose: the launch is user-paced (the OS
    //    dialog may sit open for minutes), and a concurrent pass must queue behind it
    //    rather than stack a second prompt — that is the amendment's whole point. The
    //    lock is async for exactly this reason; no other command takes it.
    let (session_dir, job_stem) = {
        let mut session_guard = state.admin_session.lock().await;
        let session = match session_guard.as_mut() {
            Some(session) => session,
            None => {
                let session = AdminSession::new()
                    .map_err(|e| format!("could not create the administrator session: {e}"))?;
                session_guard.insert(session)
            }
        };
        session.job_seq += 1;
        let stem = session.job_seq.to_string();
        let job_path = session.path().join(format!("job-{stem}.json"));
        let paths: Vec<String> = expected.iter().map(|(_, p)| p.clone()).collect();
        std::fs::write(&job_path, job_document(&paths, DEADLINE_SECS))
            .map_err(|e| format!("could not write the job file: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&job_path, std::fs::Permissions::from_mode(0o600));
        }
        let session_dir = session.path().to_path_buf();

        if session.daemon_started {
            // The whole point of the cadence: a second pass in this app run must not ask.
            log_session("reused");
        } else {
            log_session("dialog raised");
            let injected: Option<Launch> = state
                .admin_launch
                .lock()
                .map_err(|_| "the launcher seam is poisoned".to_string())?
                .clone();
            let helper: Option<PathBuf> = resolve_helper();
            let outcome = match (&injected, &helper) {
                (Some(launch), _) => launch(&session_dir),
                (None, Some(helper)) => {
                    let helper = helper.clone();
                    let dir = session_dir.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        start_daemon_with_dialog(&helper, &dir)
                    })
                    .await
                    .map_err(|e| format!("the administrator pass failed to run: {e}"))?
                }
                (None, None) => {
                    return Err(format!(
                        "could not find the {HELPER_NAME} helper: looked in \
                         $DISKGUARDIAN_ADMIN_BIN and beside the app binary"
                    ));
                }
            };

            match outcome {
                Ok(outcome) if outcome.success => {
                    // Approved: the helper must now announce itself. Approval without
                    // a ready daemon means the binary failed to start — say that, not
                    // a generic timeout.
                    if !wait_for_ready(&session_dir, ready_wait()) {
                        return Err(
                            "the administrator helper did not start after approval".to_string()
                        );
                    }
                }
                Ok(outcome) => {
                    return Err(refusal_message(&outcome.stderr));
                }
                Err(err) => {
                    return Err(format!("could not launch the administrator helper: {err}"));
                }
            }
            // Only reached on a successful start: the dialog is spent, the daemon is
            // up, and later passes must not spend another.
            session.daemon_started = true;
            log_session("approved");
        }
        (session_dir, stem)
    };

    // 4. Wait for this job's status file — the helper's completion marker — keeping
    //    the UI's status line fed from the results that arrive before it.
    let results_path = session_dir.join(format!("results-{job_stem}.jsonl"));
    let status_path = session_dir.join(format!("status-{job_stem}.json"));
    let deadline = Instant::now() + Duration::from_secs(DEADLINE_SECS + OUTER_GRACE_SECS);

    let done = Arc::new(AtomicBool::new(false));
    let total = expected.len() as u32;
    let progress_app = app.clone();
    let progress_results = results_path.clone();
    let progress_done = Arc::clone(&done);
    let poller = std::thread::spawn(move || {
        let mut last = u32::MAX;
        while !progress_done.load(Ordering::Relaxed) {
            let measured = count_lines(&progress_results);
            if measured != last {
                last = measured;
                let _ = progress_app.emit("admin-progress", AdminProgress { measured, total });
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        let measured = count_lines(&progress_results);
        if measured != last {
            let _ = progress_app.emit("admin-progress", AdminProgress { measured, total });
        }
    });

    let status_text = loop {
        if let Ok(text) = std::fs::read_to_string(&status_path) {
            break text;
        }
        if Instant::now() >= deadline {
            done.store(true, Ordering::Relaxed);
            let _ = poller.join();
            return Err("the administrator pass ran out of time; nothing was changed".to_string());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    done.store(true, Ordering::Relaxed);
    let _ = poller.join();

    let status: JobStatus = serde_json::from_str(&status_text)
        .map_err(|e| format!("the helper wrote an unreadable status file: {e}"))?;
    if status.exit != 0 {
        // The helper refused this job (schema, version, path, size). Distinct from a
        // refusal of the dialog, and the session stays usable.
        return Err(format!(
            "the measurement helper refused the job: {}",
            status
                .reason
                .unwrap_or_else(|| format!("exit {}", status.exit))
        ));
    }

    // 5. Verify each result against the path it claims, then graft. The helper ran as
    //    root over files in a directory this process owns; trusting the bytes without
    //    checking them would make that the weakest link in the tree.
    let by_path: HashMap<&str, NodeId> = expected.iter().map(|(id, p)| (p.as_str(), *id)).collect();
    let mut patched = 0u32;
    for line in read_results(&results_path) {
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
        let (view, volume) = crate::commands::project(&state, scan, None)?;
        AdminScanReport {
            scan_id: scan_id.clone(),
            patched,
            still_restricted: scan.tree.restricted_roots().len() as u32,
            bytes: scan.tree.total_bytes(),
            files: scan.tree.file_count(),
            view,
            volume,
        }
    };
    Ok(report)
}

/// A non-zero launcher exit, translated. Error -128 is AppleScript's "user canceled";
/// the message text is localized, the number is not.
fn refusal_message(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.contains("-128") || stderr.to_lowercase().contains("cancel") {
        "Administrator access was declined — restricted areas stay unmeasured".to_string()
    } else {
        format!("administrator measurement failed: {stderr}")
    }
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

/// Append one line describing a privileged pass. The one-prompt-per-session property is
/// otherwise only observable by watching the screen, which means it cannot be checked after
/// the fact by anyone who was not looking.
pub fn log_session(event: &str) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let dir = PathBuf::from(home).join("Library/Logs/DiskGuardian");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{stamp}\t{event}\n");
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("admin-session.log"))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn wait_for_ready(dir: &Path, bound: Duration) -> bool {
    let ready = dir.join("daemon.ready");
    let deadline = Instant::now() + bound;
    loop {
        if ready.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The ready wait, overridable so the IPC tests can exercise the never-ready failure
/// in milliseconds instead of half a minute. Test knob only; production uses the
/// constant.
fn ready_wait() -> Duration {
    std::env::var("DISKGUARDIAN_TEST_READY_WAIT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(READY_WAIT, Duration::from_millis)
}

/// Lines of a results file, skipping any that do not parse: a tampered file is an
/// integrity problem, not a reason to lose the whole pass.
fn read_results(results_path: &Path) -> Vec<ResultLine> {
    let Ok(text) = std::fs::read_to_string(results_path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn count_lines(results_path: &Path) -> u32 {
    let Ok(text) = std::fs::read_to_string(results_path) else {
        return 0;
    };
    text.lines().filter(|line| !line.is_empty()).count() as u32
}

/// Raise the OS dialog once and start the serving helper it approves.
///
/// The helper is backgrounded inside the dialog's shell (fixed template text), so
/// `osascript` returns as soon as the shell backgrounds it — the dialog closes, the
/// helper keeps serving. That orphaning is why the helper watches a heartbeat instead
/// of a parent pid, and why this function's job ends at "approved and ready", not at
/// "the helper has finished" (it never finishes before the app does).
#[cfg(target_os = "macos")]
fn start_daemon_with_dialog(helper: &Path, dir: &Path) -> io::Result<LaunchOutcome> {
    let source = applescript_source(&helper.display().to_string(), &dir.display().to_string())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut child = std::process::Command::new("osascript")
        .args(osascript_args(&source))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("could not run osascript: {e}")))?;
    let mut stderr_pipe = child.stderr.take();

    // The dialog is user-paced, so the wait is bounded only by the pass budget: a
    // user who leaves it up past the bound gets a typed timeout and can retry.
    let deadline = Instant::now() + Duration::from_secs(DEADLINE_SECS + OUTER_GRACE_SECS);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| io::Error::new(e.kind(), format!("waiting on osascript: {e}")))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            // Killing osascript dismisses its dialog; any helper it already started is
            // root's process, ends on its own budgets, and writes results nobody will
            // graft.
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

    if status.success() && wait_for_ready(dir, READY_WAIT) {
        return Ok(LaunchOutcome {
            success: true,
            stderr,
        });
    }
    Ok(LaunchOutcome {
        success: false,
        stderr,
    })
}

#[cfg(not(target_os = "macos"))]
fn start_daemon_with_dialog(_helper: &Path, _dir: &Path) -> io::Result<LaunchOutcome> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "administrator measurement is macOS-only for now",
    ))
}
