//! diskguardian-admin — the measurement helper behind administrator scanning.
//!
//! Capability list, in full: read job files, walk the paths they name read-only through
//! the same core walker the app uses, write one results file and one status file per
//! job into the staging directory, exit. It launches nothing, opens no sockets, writes
//! nowhere outside the staging directory, and has no code path that removes anything
//! from the disk — measurement is the whole job, and acting on what was measured stays
//! in the unprivileged app, behind the unchanged deletion guard.
//!
//! Disk-derived strings (the paths it walks) arrive as JSON inside job files and leave
//! as JSON inside results files; none ever reaches an argument vector that this program
//! builds, because it builds none. The elevated launcher in the app interpolates only
//! two app-generated paths into a fixed template.
//!
//! Two modes:
//!
//! - `--job <file>`: one shot. One job in, `results.jsonl` + `status.json` beside it.
//!   This is the whole protocol a future SMAppService runner needs, and it is how the
//!   suite exercises validation without a dialog.
//! - `--serve <dir>`: one privileged process per app session (the principal's
//!   2026-11-02 amendment: one authorisation prompt per session, not per pass). The
//!   app stages `job-<n>.json` files into `<dir>`; this process picks them up in
//!   order, writes `results-<n>.jsonl` and finally `status-<n>.json` per job, and
//!   keeps doing so until the session ends.
//!
//! # How the serving process knows the app is gone
//!
//! It cannot watch a parent pid: the launcher backgrounds this process on purpose
//! (`do shell script "… &"` returns immediately so the dialog closes), which orphans
//! it to launchd within milliseconds. kqueue or a parent-death signal would need
//! `unsafe` (forbidden in this crate) or a platform dependency this helper refuses to
//! grow. So liveness is a heartbeat the app owns: it touches `<dir>/session.alive`
//! every few seconds while it lives, and this process exits when that file goes stale.
//! Three exits, each its own test: the heartbeat goes stale (app crashed), the
//! staging directory disappears (app exited cleanly — the directory is the app's
//! temp dir and is removed with it), or nothing has asked for work for the idle
//! budget (the session outlived its usefulness).
//!
//! It runs correctly without elevation too — it simply reports `restricted` for
//! anything it cannot read — which is what makes the whole protocol testable without
//! an administrator password.

use diskguardian_core::{scan, Node, NodeKind, ScanOptions, Tree, DEADLINE_NOTE};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

/// Distinct exit codes, so the caller can tell a refused job from a failed run.
mod code {
    /// The job ran (or the session ended normally); per-path outcomes (including
    /// `restricted`) live in the results file.
    pub const RAN: u8 = 0;
    /// Wrong invocation.
    pub const USAGE: u8 = 2;
    /// The job file could not be read, parsed, or set up for output.
    pub const BAD_JOB: u8 = 3;
    /// The job speaks a version this helper does not.
    pub const BAD_VERSION: u8 = 4;
    /// A job path failed validation.
    pub const BAD_PATH: u8 = 5;
    /// The job file is absurdly large; no honest job of ours looks like that.
    pub const TOO_BIG: u8 = 6;
}

const JOB_VERSION: u64 = 1;
/// One budget for the whole job, not per path: the batch is one prompt, one pass.
/// Clamped at a day so a hostile `deadline_secs` cannot overflow `Instant` addition.
const MAX_DEADLINE_SECS: u64 = 86_400;
/// A job names tens of paths at ~100 bytes each; one mebibyte is four orders of
/// magnitude of headroom, and anything beyond it is not a job we wrote.
const MAX_JOB_BYTES: usize = 1024 * 1024;

const READY_FILE: &str = "daemon.ready";
const ALIVE_FILE: &str = "session.alive";
/// How often the serving loop looks for work, and the default budgets the app can
/// override for testing.
const POLL_EVERY: Duration = Duration::from_millis(200);
const DEFAULT_IDLE_SECS: u64 = 15 * 60;
const DEFAULT_STALE_SECS: u64 = 30;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    /// Checked on the raw JSON before this struct is built, so a wrong version exits
    /// with its own code rather than the unknown-fields one. Still declared here:
    /// `deny_unknown_fields` rejects the document outright if the field is missing.
    #[allow(dead_code)]
    version: u64,
    deadline_secs: u64,
    paths: Vec<String>,
}

#[derive(Serialize)]
struct ResultLine {
    version: u64,
    path: String,
    status: &'static str,
    tree: Tree,
}

#[derive(Serialize)]
struct StatusFile {
    version: u64,
    exit: u8,
    completed: usize,
    deadline_hit: bool,
    /// Why a job was refused, in serving mode where there is no exit code to read.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(run())
}

#[derive(Debug)]
enum Mode {
    Usage,
    OneShot(PathBuf),
    Serve {
        dir: PathBuf,
        idle_secs: u64,
        stale_secs: u64,
    },
}

fn parse_args(args: &[std::ffi::OsString]) -> Mode {
    let flag = args.first().map(std::ffi::OsStr::new);
    match flag {
        Some(f) if f == "--job" && args.len() == 2 => Mode::OneShot(PathBuf::from(&args[1])),
        Some(f) if f == "--serve" && args.len() >= 2 => {
            let dir = PathBuf::from(&args[1]);
            let mut idle_secs = DEFAULT_IDLE_SECS;
            let mut stale_secs = DEFAULT_STALE_SECS;
            let mut rest = &args[2..];
            while let Some((head, tail)) = rest.split_first() {
                let value = tail.first().and_then(|v| v.to_str()).and_then(|v| v.parse().ok());
                match (head.to_str(), value) {
                    (Some("--idle-secs"), Some(v)) => idle_secs = v,
                    (Some("--stale-secs"), Some(v)) => stale_secs = v,
                    _ => return Mode::Usage,
                }
                rest = &tail[1..];
            }
            Mode::Serve { dir, idle_secs, stale_secs }
        }
        _ => Mode::Usage,
    }
}

fn run() -> u8 {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match parse_args(&args) {
        Mode::Usage => {
            eprintln!("usage: diskguardian-admin --job <job.json> | --serve <dir> [--idle-secs N] [--stale-secs N]");
            code::USAGE
        }
        Mode::OneShot(job_path) => one_shot(&job_path),
        Mode::Serve { dir, idle_secs, stale_secs } => serve(&dir, idle_secs, stale_secs),
    }
}

/// The one-shot entry: process one job file, writing `results.jsonl`/`status.json`
/// beside it. A refusal writes its status (with the reason) and returns the code.
fn one_shot(job_path: &Path) -> u8 {
    let dir = job_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let results_path = dir.join("results.jsonl");
    let status_path = dir.join("status.json");
    match run_one_job(job_path, &results_path, &status_path) {
        Ok(()) => code::RAN,
        Err((exit_code, reason)) => {
            eprintln!("{reason}");
            write_status(&status_path, exit_code, 0, false, Some(&reason));
            world_readable(&status_path);
            exit_code
        }
    }
}

/// The serving entry: one process per app session, jobs until the session ends.
fn serve(dir: &Path, idle_secs: u64, stale_secs: u64) -> u8 {
    let idle = Duration::from_secs(idle_secs);
    let stale = Duration::from_secs(stale_secs);
    let started = Instant::now();
    let mut last_activity = started;
    let mut processed: HashSet<std::ffi::OsString> = HashSet::new();

    // Announce readiness: the app waits for this file to know the dialog was approved
    // and the helper actually came up, rather than guessing from the launcher's exit.
    let ready = dir.join(READY_FILE);
    #[derive(Serialize)]
    struct Ready<'a> {
        version: u64,
        pid: u32,
        ready: &'a str,
    }
    let _ = std::fs::write(
        &ready,
        serde_json::to_vec(&Ready {
            version: JOB_VERSION,
            pid: std::process::id(),
            ready: "waiting for jobs",
        })
        .unwrap_or_default(),
    );
    world_readable(&ready);

    loop {
        // Clean exit: the app's staging directory is a temp dir owned by the app; its
        // disappearance is the shutdown signal.
        if !dir.is_dir() {
            break;
        }
        // Crash: the heartbeat went stale. Missing file counts from process start,
        // with double budget, because the app touches it before launching this
        // process — a file that never appeared means the app died before first touch.
        if heartbeat_stale(dir, started, stale) {
            break;
        }

        // Jobs, in name order so two writers cannot reorder a session.
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut names: Vec<std::ffi::OsString> = entries
                .flatten()
                .map(|e| e.file_name())
                .filter(|name| {
                    name.to_str().is_some_and(|n| n.starts_with("job-") && n.ends_with(".json"))
                })
                .collect();
            names.sort();
            for name in names {
                if !processed.insert(name.clone()) {
                    continue;
                }
                last_activity = Instant::now();
                let stem = name
                    .to_str()
                    .and_then(|n| n.strip_prefix("job-"))
                    .and_then(|n| n.strip_suffix(".json"))
                    .unwrap_or("unknown");
                let results_path = dir.join(format!("results-{stem}.jsonl"));
                let status_path = dir.join(format!("status-{stem}.json"));
                if let Err((exit_code, reason)) =
                    run_one_job(&dir.join(&name), &results_path, &status_path)
                {
                    eprintln!("job refused: {reason}");
                    // A refused job must not end the session: report it per job, in
                    // the file the app is polling, and keep serving the honest ones.
                    write_status(&status_path, exit_code, 0, false, Some(&reason));
                    world_readable(&status_path);
                }
            }
        }

        // Usefulness: nobody has asked for work in the idle budget.
        if last_activity.elapsed() > idle {
            break;
        }
        std::thread::sleep(POLL_EVERY);
    }

    code::RAN
}

fn heartbeat_stale(dir: &Path, started: Instant, stale: Duration) -> bool {
    match std::fs::metadata(dir.join(ALIVE_FILE)).and_then(|m| m.modified()) {
        Ok(touched) => touched
            .elapsed()
            .map(|age| age > stale)
            // A clock that cannot be read backwards is not a reason to exit.
            .unwrap_or(false),
        Err(_) => started.elapsed() > stale.checked_mul(2).unwrap_or(stale),
    }
}

/// Validate and run one job: read, size-check, version-check, schema-check, validate
/// every path, then walk and write results + a final status file. The status file is
/// the completion marker — it is written last, after every results flush, so a reader
/// that sees it may trust the results beside it.
fn run_one_job(
    job_path: &Path,
    results_path: &Path,
    status_path: &Path,
) -> Result<(), (u8, String)> {
    let raw = std::fs::read(job_path)
        .map_err(|e| (code::BAD_JOB, format!("cannot read job file {}: {e}", job_path.display())))?;
    if raw.len() > MAX_JOB_BYTES {
        return Err((
            code::TOO_BIG,
            format!(
                "job file is {} bytes; the cap is {MAX_JOB_BYTES}, and no honest job looks like that",
                raw.len()
            ),
        ));
    }

    // Version first, separately: an old helper meeting a new job must refuse for the
    // version reason, not for the unknown-fields reason.
    let parsed: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| (code::BAD_JOB, format!("job file is not valid JSON: {e}")))?;
    if parsed["version"].as_u64() != Some(JOB_VERSION) {
        return Err((
            code::BAD_VERSION,
            format!(
                "job version {} is not supported: this helper speaks version {JOB_VERSION}",
                parsed["version"]
            ),
        ));
    }
    let job: Job = serde_json::from_value(parsed).map_err(|e| {
        // Unknown fields land here: the job schema is closed on purpose.
        (code::BAD_JOB, format!("job file does not match the version-{JOB_VERSION} schema: {e}"))
    })?;

    // Validate every path before walking any, so a bad job cannot leave a
    // half-measured tree behind.
    for path in &job.paths {
        validate_path(Path::new(path))
            .map_err(|reason| (code::BAD_PATH, format!("job path refused: {path} ({reason})")))?;
    }

    let mut results = std::fs::File::create(results_path).map_err(|e| {
        (code::BAD_JOB, format!("cannot write {}: {e}", results_path.display()))
    })?;

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(job.deadline_secs.min(MAX_DEADLINE_SECS)))
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(MAX_DEADLINE_SECS));

    let mut completed = 0usize;
    let mut deadline_hit = false;
    for path in &job.paths {
        let tree = measure(Path::new(path), deadline);
        let hit = tree
            .nodes
            .iter()
            .any(|node| node.note.as_deref() == Some(DEADLINE_NOTE));
        deadline_hit |= hit;
        let status = if hit {
            "deadline"
        } else if tree.nodes[tree.root].kind == NodeKind::Restricted {
            // Still unreadable even elevated: the TCC case. Reported honestly, and
            // counted by the app as still-restricted rather than measured.
            "restricted"
        } else {
            "ok"
        };

        let line = ResultLine {
            version: JOB_VERSION,
            path: path.clone(),
            status,
            tree,
        };
        let encoded = serde_json::to_string(&line).unwrap_or_else(|_| "{}".to_string());
        writeln!(results, "{encoded}")
            .map_err(|e| (code::BAD_JOB, format!("cannot append a result for {path}: {e}")))?;
        let _ = results.flush();
        completed += 1;
    }

    write_status(status_path, code::RAN, completed, deadline_hit, None);
    // This process runs as root; the app that reads these files does not.
    world_readable(results_path);
    world_readable(status_path);
    Ok(())
}

/// Write a status file whole, via a temp name and a rename, so a reader never sees a
/// half-written marker.
fn write_status(
    status_path: &Path,
    exit: u8,
    completed: usize,
    deadline_hit: bool,
    reason: Option<&str>,
) {
    let status = StatusFile {
        version: JOB_VERSION,
        exit,
        completed,
        deadline_hit,
        reason: reason.map(str::to_string),
    };
    let Ok(encoded) = serde_json::to_vec(&status) else {
        return;
    };
    let tmp = status_path.with_extension("json.tmp");
    if std::fs::write(&tmp, encoded).is_ok() {
        let _ = std::fs::rename(&tmp, status_path);
    }
}

/// Every job path must be absolute, free of `..`, and present. Walking any absolute
/// path is not a compromise — this helper only measures — but a job that names
/// nonsense is refused rather than half-run.
fn validate_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("must be absolute");
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("must not contain '..'");
    }
    if std::fs::symlink_metadata(path).is_err() {
        return Err("does not exist");
    }
    Ok(())
}

fn measure(path: &Path, deadline: Instant) -> Tree {
    let opts = ScanOptions {
        max_entries: None,
        deadline: Some(deadline),
    };
    match scan(path, &opts) {
        Ok(tree) => tree,
        // The path existed at validation time and vanished before the walk, or the
        // root itself cannot be stat'd: the same reported-not-dropped shape as any
        // unreadable directory (A3), as a one-node tree.
        Err(err) => Tree {
            nodes: vec![Node {
                name: path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
                kind: NodeKind::Restricted,
                own_bytes: 0,
                total_bytes: 0,
                file_count: 0,
                children: Vec::new(),
                parent: None,
                partial: true,
                link_skipped: false,
                missing: false,
                note: Some(err.to_string()),
            }],
            root: 0,
            scanned_root: path.to_path_buf(),
            duration_ms: 0,
            errors: 1,
        },
    }
}

#[cfg(unix)]
fn world_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
}

#[cfg(not(unix))]
fn world_readable(_path: &Path) {}
