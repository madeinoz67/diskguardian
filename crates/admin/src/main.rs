//! diskguardian-admin — the one-shot measurement helper behind administrator scanning.
//!
//! Capability list, in full: read a job file, walk the paths it names read-only through
//! the same core walker the app uses, append one JSON line per completed path to
//! `results.jsonl`, write `status.json`, exit. It launches nothing, opens no sockets,
//! writes nowhere outside the job's own directory, and has no code path that removes
//! anything from the disk — measurement is the whole job, and acting on what was
//! measured stays in the unprivileged app, behind the unchanged deletion guard.
//!
//! Disk-derived strings (the paths it walks) arrive as JSON inside the job file and
//! leave as JSON inside the results file; none ever reaches an argument vector that
//! this program builds, because it builds none. The elevated launcher in the app
//! interpolates only two app-generated paths into a fixed template.
//!
//! It runs correctly without elevation too — it simply reports `restricted` for
//! anything it cannot read — which is what makes the whole protocol testable without
//! an administrator password.

use diskguardian_core::{scan, Node, NodeKind, ScanOptions, Tree, DEADLINE_NOTE};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

/// Distinct exit codes, so the caller can tell a refused job from a failed run.
mod code {
    /// The job ran; per-path outcomes (including `restricted`) live in the results file.
    pub const RAN: u8 = 0;
    /// Wrong invocation.
    pub const USAGE: u8 = 2;
    /// The job file could not be read, parsed, or set up for output.
    pub const BAD_JOB: u8 = 3;
    /// The job speaks a version this helper does not.
    pub const BAD_VERSION: u8 = 4;
    /// A job path failed validation.
    pub const BAD_PATH: u8 = 5;
}

const JOB_VERSION: u64 = 1;
/// One budget for the whole job, not per path: the batch is one prompt, one pass.
/// Clamped at a day so a hostile `deadline_secs` cannot overflow `Instant` addition.
const MAX_DEADLINE_SECS: u64 = 86_400;

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
}

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(run())
}

fn run() -> u8 {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--job" {
        eprintln!("usage: diskguardian-admin --job <job.json>");
        return code::USAGE;
    }
    let job_path = PathBuf::from(&args[1]);

    let raw = match std::fs::read(&job_path) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("cannot read job file {}: {err}", job_path.display());
            return code::BAD_JOB;
        }
    };

    // Version first, separately: an old helper meeting a new job must refuse for the
    // version reason, not for the unknown-fields reason.
    let parsed: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("job file is not valid JSON: {err}");
            return code::BAD_JOB;
        }
    };
    if parsed["version"].as_u64() != Some(JOB_VERSION) {
        eprintln!(
            "job version {} is not supported: this helper speaks version {JOB_VERSION}",
            parsed["version"]
        );
        return code::BAD_VERSION;
    }
    let job: Job = match serde_json::from_value(parsed) {
        Ok(job) => job,
        Err(err) => {
            // Unknown fields land here: the job schema is closed on purpose.
            eprintln!("job file does not match the version-{JOB_VERSION} schema: {err}");
            return code::BAD_JOB;
        }
    };

    // Validate every path before walking any, so a bad job cannot leave a half-measured
    // tree behind.
    for path in &job.paths {
        if let Err(reason) = validate_path(Path::new(path)) {
            eprintln!("job path refused: {path} ({reason})");
            return code::BAD_PATH;
        }
    }

    let job_dir = job_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let results_path = job_dir.join("results.jsonl");
    let status_path = job_dir.join("status.json");

    let mut results = match std::fs::File::create(&results_path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("cannot write {}: {err}", results_path.display());
            return code::BAD_JOB;
        }
    };

    let deadline = Instant::now()
        .checked_add(Duration::from_secs(
            job.deadline_secs.min(MAX_DEADLINE_SECS),
        ))
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
        if let Err(err) = writeln!(results, "{encoded}") {
            eprintln!("cannot append a result for {path}: {err}");
        }
        let _ = results.flush();
        completed += 1;
    }

    let status = StatusFile {
        version: JOB_VERSION,
        exit: code::RAN,
        completed,
        deadline_hit,
    };
    if let Ok(encoded) = serde_json::to_vec(&status) {
        if let Err(err) = std::fs::write(&status_path, encoded) {
            eprintln!("cannot write {}: {err}", status_path.display());
        }
    }

    // This process runs as root; the app that reads these files does not.
    world_readable(&results_path);
    world_readable(&status_path);

    code::RAN
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
