//! The helper's contract, driven the way the app drives it: a job file path as the
//! only argument, results and status files in the job's directory, distinct exit
//! codes for every way a job can be refused.
//!
//! The helper runs unprivileged in these tests and that is half the point: the
//! protocol is launcher-agnostic, and the unprivileged behaviour (report `restricted`,
//! recover nothing) is the control arm of the elevated test below.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;

const BIN: &str = env!("CARGO_BIN_EXE_diskguardian-admin");

fn run(args: &[&str]) -> Output {
    std::process::Command::new(BIN)
        .args(args)
        .env_clear()
        .output()
        .expect("run diskguardian-admin")
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// A job directory with a job file naming `paths`, returning the job file's path.
fn job(paths: &[&str], deadline_secs: u64) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let quoted: Vec<String> = paths.iter().map(|p| format!("\"{p}\"")).collect();
    let body = format!(
        "{{\"version\":1,\"deadline_secs\":{deadline_secs},\"paths\":[{}]}}",
        quoted.join(",")
    );
    let job_path = dir.path().join("job.json");
    fs::write(&job_path, body).expect("write job");
    (dir, job_path)
}

fn results(job_path: &Path) -> Vec<serde_json::Value> {
    let text = fs::read_to_string(job_path.parent().expect("parent").join("results.jsonl"))
        .expect("results.jsonl exists");
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is one JSON object"))
        .collect()
}

fn status(job_path: &Path) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(job_path.parent().expect("parent").join("status.json"))
            .expect("status.json exists"),
    )
    .expect("status is JSON")
}

fn fixture_readme(files: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..files {
        fs::write(
            dir.path().join(format!("payload-{i:03}.bin")),
            vec![7u8; 64 * 1024],
        )
        .expect("write");
    }
    dir
}

/// A directory this process cannot enter, holding 64 KiB × `files` of payload.
fn fixture_locked(files: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).expect("mkdir");
    for i in 0..files {
        fs::write(
            locked.join(format!("payload-{i:03}.bin")),
            vec![7u8; 64 * 1024],
        )
        .expect("write");
    }
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    (dir, locked)
}

fn unlock(path: &Path) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
}

fn uid() -> u32 {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(501)
}

#[test]
fn wrong_usage_exits_two() {
    let bare = run(&[]);
    assert_eq!(code(&bare), 2, "no arguments: {}", stderr_of(&bare));

    let partial = run(&["--job"]);
    assert_eq!(
        code(&partial),
        2,
        "flag without a path: {}",
        stderr_of(&partial)
    );

    let extra = run(&["--job", "/tmp/x", "surplus"]);
    assert_eq!(code(&extra), 2, "a second operand: {}", stderr_of(&extra));
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_job_file_that_cannot_be_read_exits_three() {
    let out = run(&["--job", "/nonexistent/job.json"]);
    assert_eq!(code(&out), 3, "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("job"),
        "the refusal must name what it could not read"
    );
}

#[test]
fn an_unknown_version_is_refused_before_anything_is_walked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = fixture_readme(2);
    let job_path = dir.path().join("job.json");
    fs::write(
        &job_path,
        format!(
            "{{\"version\":2,\"deadline_secs\":60,\"paths\":[\"{}\"]}}",
            target.path().display()
        ),
    )
    .expect("write job");

    let out = run(&["--job", job_path.to_str().expect("utf8")]);
    assert_eq!(code(&out), 4, "{}", stderr_of(&out));
    assert!(
        !job_path
            .parent()
            .expect("parent")
            .join("results.jsonl")
            .exists(),
        "a refused job must walk nothing"
    );
}

#[test]
fn an_unknown_field_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = fixture_readme(1);
    let job_path = dir.path().join("job.json");
    fs::write(
        &job_path,
        format!(
            "{{\"version\":1,\"deadline_secs\":60,\"paths\":[\"{}\"],\"surprise\":true}}",
            target.path().display()
        ),
    )
    .expect("write job");

    let out = run(&["--job", job_path.to_str().expect("utf8")]);
    assert_eq!(code(&out), 3, "{}", stderr_of(&out));
    assert!(
        !job_path
            .parent()
            .expect("parent")
            .join("results.jsonl")
            .exists(),
        "a refused job must walk nothing"
    );
}

#[test]
fn malformed_paths_are_refused_before_anything_is_walked() {
    let readable = fixture_readme(1);
    let root = readable.path().display().to_string();

    // Shape only: relative and traversing paths are malformed jobs.
    for (label, path) in [
        ("relative", "relative/path".to_string()),
        ("dotdot", format!("{root}/../..")),
    ] {
        let body = format!("{{\"version\":1,\"deadline_secs\":60,\"paths\":[\"{path}\"]}}");
        let dir = tempfile::tempdir().expect("tempdir");
        let job_path = dir.path().join("job.json");
        fs::write(&job_path, body).expect("write job");

        let out = run(&["--job", job_path.to_str().expect("utf8")]);
        assert_eq!(code(&out), 5, "{label}: {}", stderr_of(&out));
        assert!(
            !job_path
                .parent()
                .expect("parent")
                .join("results.jsonl")
                .exists(),
            "{label}: a refused job must walk nothing"
        );
    }
}

/// A path that cannot be read is a RESULT, not a malformed job.
///
/// On macOS a SIP-protected root answers EPERM even to root, and `symlink_metadata` reports
/// that identically to "missing". Refusing the job for it once failed a 57-path pass because
/// one root was invisible, which is the opposite of the rule that unmeasurable space is
/// reported rather than dropped.
#[test]
fn an_unreadable_path_becomes_a_restricted_result_not_a_refusal() {
    let readable = fixture_readme(2);
    let missing = format!("{}/no-such-child", readable.path().display());

    let (dir, job_path) = job(&[readable.path().to_str().expect("utf8"), &missing], 60);
    let out = run(&["--job", job_path.to_str().expect("utf8")]);

    assert_eq!(code(&out), 0, "the job must complete: {}", stderr_of(&out));

    let results = fs::read_to_string(dir.path().join("results.jsonl")).expect("results");
    let lines: Vec<&str> = results.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "one result per path, including the unreadable one"
    );

    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("json");
    assert_eq!(
        second["status"], "restricted",
        "the unreadable path is reported as restricted: {second}"
    );

    let status: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("status.json")).expect("status"))
            .expect("json");
    assert_eq!(status["completed"], 2, "both paths produced a result");
}

#[test]
fn a_valid_job_measures_paths_and_writes_both_files() {
    let target = fixture_readme(4);
    let (dir, job_path) = job(&[target.path().to_str().expect("utf8")], 60);

    let out = run(&["--job", job_path.to_str().expect("utf8")]);
    assert_eq!(code(&out), 0, "{}", stderr_of(&out));

    let lines = results(&job_path);
    assert_eq!(lines.len(), 1, "one line per completed path");
    assert_eq!(lines[0]["version"], 1);
    assert_eq!(lines[0]["path"], target.path().display().to_string());
    assert_eq!(lines[0]["status"], "ok");
    let bytes = lines[0]["tree"]["nodes"][0]["total_bytes"]
        .as_u64()
        .expect("total as u64");
    assert!(
        bytes >= 4 * 64 * 1024,
        "four 64 KiB payloads measured, got {bytes}"
    );

    let st = status(&job_path);
    assert_eq!(st["version"], 1);
    assert_eq!(st["exit"], 0);
    assert_eq!(st["completed"], 1);
    assert_eq!(st["deadline_hit"], false);

    // It runs as root in production; the unprivileged app must be able to read both.
    for name in ["results.jsonl", "status.json"] {
        let mode = fs::metadata(dir.path().join(name))
            .expect("file present")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644, "{name} must be readable by the app");
    }
}

#[test]
fn an_unreadable_path_is_reported_restricted_not_measured() {
    if uid() == 0 {
        eprintln!("skipping: running as root, permissions cannot be tested");
        return;
    }
    let (fixture, locked) = fixture_locked(16);
    let (_job, job_path) = job(&[locked.to_str().expect("utf8")], 60);

    let out = run(&["--job", job_path.to_str().expect("utf8")]);
    assert_eq!(
        code(&out),
        0,
        "an unreadable path is a result, not a failure"
    );

    let lines = results(&job_path);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["status"], "restricted");
    let nodes = lines[0]["tree"]["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 1, "a single Restricted node, nothing invented");
    assert_eq!(nodes[0]["kind"], "restricted");
    let bytes = nodes[0]["total_bytes"].as_u64().expect("bytes");
    assert!(
        bytes < 64 * 1024,
        "unprivileged recovery must be nothing but the directory's own metadata, got {bytes}"
    );

    unlock(&locked);
    drop(fixture);
}

#[test]
fn a_spent_deadline_reports_deadline_with_partial_results() {
    let target = fixture_readme(2);
    let (dir, job_path) = job(&[target.path().to_str().expect("utf8")], 0);

    let out = run(&["--job", job_path.to_str().expect("utf8")]);
    assert_eq!(code(&out), 0, "{}", stderr_of(&out));

    let lines = results(&job_path);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["status"], "deadline");
    assert_eq!(
        lines[0]["tree"]["nodes"][0]["note"], "deadline reached",
        "the note must say why it is unmeasured"
    );

    let st = status(&job_path);
    assert_eq!(st["deadline_hit"], true);
    assert_eq!(st["completed"], 1);
    assert!(
        dir.path().join("results.jsonl").exists(),
        "deadline results are still written"
    );
}

/// The review gate, made mechanical: the helper's whole capability list is read-job,
/// walk-read-only, write-two-files, exit. If any removal, process-spawning or network
/// call site ever appears in its source, this fails before a reviewer has to look.
/// Since the lib+bin split (multi-platform release design §3.7.1) the protocol lives in
/// `src/lib.rs` — shared with the Windows app binary's elevated serve branch — so the
/// guard reads every source file the serve path can be built from, not just the
/// sidecar's argv wrapper.
#[test]
fn helper_source_has_no_removal_process_or_network_callsites() {
    let mut src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("read own source");
    src.push_str(
        &fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
            .expect("read own library source"),
    );
    for needle in [
        "core::delete",
        "::delete",
        "remove_file",
        "remove_dir",
        "trash",
        "Command",
        "std::net",
        "TcpStream",
        "UdpSocket",
        "curl",
    ] {
        assert!(
            !src.contains(needle),
            "the helper must never contain {needle:?}: measurement is its whole capability list"
        );
    }
}

/// AC2: as root the helper agrees with `du` on a POSIX-denied tree; unprivileged, the
/// identical pipeline recovers nothing (proving the effect is elevation, not a re-walk
/// artifact). Gated on passwordless sudo, skipped otherwise, mirroring the skip-as-root
/// guard in `crates/core/tests/scan.rs`.
#[cfg(target_os = "macos")]
#[test]
fn elevated_helper_agrees_with_du_and_unprivileged_recovers_nothing() {
    if uid() == 0 {
        eprintln!("skipping: running as root, the unprivileged control would be meaningless");
        return;
    }
    let probe = std::process::Command::new("sudo")
        .args(["-n", "true"])
        .status();
    if !matches!(&probe, Ok(s) if s.success()) {
        eprintln!("skipping: sudo -n unavailable, the elevated arm needs it");
        return;
    }

    let (fixture, locked) = fixture_locked(128); // 8 MiB behind mode 000
    let locked_str = locked.to_str().expect("utf8").to_string();

    // Elevated arm.
    let (elev_dir, elev_job) = job(&[&locked_str], 120);
    let elevated = std::process::Command::new("sudo")
        .args(["-n", BIN, "--job"])
        .arg(&elev_job)
        .output()
        .expect("run elevated helper");
    assert_eq!(
        code(&elevated),
        0,
        "elevated run: {}",
        String::from_utf8_lossy(&elevated.stderr)
    );
    let lines = results(&elev_job);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0]["status"],
        "ok",
        "root reads through mode 000: {}",
        serde_json::to_string(&lines[0]).unwrap_or_default()
    );
    let measured = lines[0]["tree"]["nodes"][0]["total_bytes"]
        .as_u64()
        .expect("measured bytes");

    let du = std::process::Command::new("sudo")
        .args(["-n", "du", "-sk"])
        .arg(&locked)
        .output()
        .expect("run du");
    assert!(du.status.success(), "du failed");
    let kb: u64 = String::from_utf8_lossy(&du.stdout)
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .expect("du kilobytes");
    let du_bytes = kb * 1024;
    assert!(
        (measured as i128 - du_bytes as i128).abs() <= du_bytes as i128 / 100,
        "helper said {measured}, du said {du_bytes} (1% tolerance)"
    );

    // Unprivileged control: same pipeline, no elevation, must recover nothing.
    let (_plain_dir, plain_job) = job(&[&locked_str], 120);
    let plain = run(&["--job", plain_job.to_str().expect("utf8")]);
    assert_eq!(code(&plain), 0);
    let control = results(&plain_job);
    assert_eq!(control[0]["status"], "restricted");
    let recovered = control[0]["tree"]["nodes"][0]["total_bytes"]
        .as_u64()
        .expect("control bytes");
    assert!(
        recovered < measured / 10,
        "the unprivileged control recovered {recovered} of {measured}: the effect would not be elevation"
    );

    unlock(&locked);
    drop(fixture);
    drop(elev_dir);
}
