//! The serving mode: one privileged process per app session, many jobs.
//!
//! The principal's amendment (2026-11-02) changed the helper's lifetime: one OS
//! authorisation prompt per app run, then job files are served until the session ends.
//! A long-lived root process is a bigger window than a one-shot, so these tests pin
//! the two properties that window depends on: it exits (three ways: clean app exit,
//! dead app, idle), and it refuses jobs it cannot trust (malformed, oversized,
//! unwalkable) without walking anything.
//!
//! All of this runs unprivileged — the helper is launcher-agnostic, and its lifecycle
//! behaviour does not depend on being root.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_diskguardian-admin");
/// Wall-clock bounds are generous multiples of the configured budgets, because the
/// property under test is "it exits", not "it exits in exactly N seconds".
const WAIT: Duration = Duration::from_secs(10);

fn spawn_serve(dir: &Path, idle_secs: u64, stale_secs: u64) -> Child {
    Command::new(BIN)
        .args([
            "--serve",
            dir.to_str().expect("utf8 dir"),
            "--idle-secs",
            &idle_secs.to_string(),
            "--stale-secs",
            &stale_secs.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn diskguardian-admin --serve")
}

/// Poll until the child exits, or `WAIT` passes (returns None).
fn wait_exit(child: &mut Child) -> Option<ExitStatus> {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().expect("child is alive") {
            return Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_ready(dir: &Path) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if dir.join("daemon.ready").exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Drop a job file into the staging directory, the way the app does.
fn drop_job(dir: &Path, id: &str, paths: &[&str]) {
    let quoted: Vec<String> = paths.iter().map(|p| format!("\"{p}\"")).collect();
    let body = format!(
        "{{\"version\":1,\"deadline_secs\":60,\"paths\":[{}]}}",
        quoted.join(",")
    );
    fs::write(dir.join(format!("job-{id}.json")), body).expect("write job");
}

fn wait_status(dir: &Path, id: &str) -> serde_json::Value {
    let path = dir.join(format!("status-{id}.json"));
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(text) = fs::read_to_string(&path) {
            return serde_json::from_str(&text).expect("status is JSON");
        }
        assert!(Instant::now() < deadline, "status-{id}.json never appeared");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn readable_fixture(files: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..files {
        fs::write(dir.path().join(format!("p-{i}.bin")), vec![7u8; 64 * 1024]).expect("write");
    }
    dir
}

/// The heartbeat the app writes while it lives. One touch stands in for "the app is
/// alive"; the staleness tests let it age.
fn touch_alive(dir: &Path) {
    fs::write(dir.join("session.alive"), b"1").expect("heartbeat");
}

#[test]
fn serve_mode_processes_jobs_as_they_appear() {
    let stage = tempfile::tempdir().expect("tempdir");
    let fixture = readable_fixture(2);
    let mut child = spawn_serve(stage.path(), 900, 900);
    touch_alive(stage.path());

    assert!(wait_ready(stage.path()), "the daemon must announce itself");
    drop_job(stage.path(), "1", &[fixture.path().to_str().expect("utf8")]);

    let status = wait_status(stage.path(), "1");
    assert_eq!(status["exit"], 0, "the job completes: {status}");
    let results = fs::read_to_string(stage.path().join("results-1.jsonl")).expect("results");
    let line: serde_json::Value =
        serde_json::from_str(results.lines().next().expect("one line")).expect("line is JSON");
    assert_eq!(line["status"], "ok");
    assert!(
        line["tree"]["nodes"][0]["total_bytes"]
            .as_u64()
            .expect("bytes")
            >= 2 * 64 * 1024
    );

    // A second job, same daemon, no new process anywhere.
    drop_job(stage.path(), "2", &[fixture.path().to_str().expect("utf8")]);
    let second = wait_status(stage.path(), "2");
    assert_eq!(second["exit"], 0, "the daemon serves again: {second}");

    // Clean shutdown: the app's staging directory is removed on exit; the daemon
    // must notice and leave.
    drop(stage);
    let exited = wait_exit(&mut child).expect("the daemon exits when the session dir goes");
    assert!(exited.success(), "lifecycle exits are clean: {exited}");
}

/// Demanded test 1a: kill the app (here: remove the staging directory, which is what
/// the app's TempDir drop does on exit) and the privileged process is gone.
#[test]
fn serve_mode_exits_when_the_session_directory_disappears() {
    let stage = tempfile::tempdir().expect("tempdir");
    let mut child = spawn_serve(stage.path(), 900, 900);
    touch_alive(stage.path());
    assert!(wait_ready(stage.path()));

    drop(stage);
    let exited = wait_exit(&mut child);
    assert!(exited.is_some(), "a daemon that outlives its app is the failure mode");
}

/// Demanded test 1b: the app crashes (no clean drop), so the heartbeat goes stale.
#[test]
fn serve_mode_exits_when_the_heartbeat_goes_stale() {
    let stage = tempfile::tempdir().expect("tempdir");
    let mut child = spawn_serve(stage.path(), 900, 1);
    touch_alive(stage.path());
    assert!(wait_ready(stage.path()));
    // And then the "app" never touches it again.

    let exited = wait_exit(&mut child).expect("a stale heartbeat must end the daemon");
    assert!(exited.success());
}

/// Demanded test 1c: nothing to do, nobody alive to ask — it stops itself.
#[test]
fn serve_mode_exits_after_the_idle_timeout() {
    let stage = tempfile::tempdir().expect("tempdir");
    // A fresh heartbeat, so staleness cannot be what ends it: only idleness may.
    let mut child = spawn_serve(stage.path(), 1, 900);
    touch_alive(stage.path());
    assert!(wait_ready(stage.path()));

    let exited = wait_exit(&mut child).expect("an idle daemon must retire");
    assert!(exited.success());
}

/// The negative control for the lifecycle tests: while the heartbeat is fresh and
/// jobs arrive, the daemon must still be there. A watchdog that simply kills the
/// daemon unconditionally would pass every exit test.
#[test]
fn serve_mode_stays_while_the_heartbeat_is_fresh_and_jobs_arrive() {
    let stage = tempfile::tempdir().expect("tempdir");
    let fixture = readable_fixture(1);
    let mut child = spawn_serve(stage.path(), 900, 1);
    touch_alive(stage.path());
    assert!(wait_ready(stage.path()));

    // ~3 s of fresh heartbeats against a 1 s staleness budget.
    let until = Instant::now() + Duration::from_secs(3);
    let mut id = 0;
    while Instant::now() < until {
        touch_alive(stage.path());
        id += 1;
        drop_job(stage.path(), &id.to_string(), &[fixture.path().to_str().expect("utf8")]);
        wait_status(stage.path(), &id.to_string());
    }
    assert!(
        child.try_wait().expect("alive").is_none(),
        "a fresh heartbeat and live work must keep the daemon running"
    );

    // Then stop touching it, and it leaves: the same child, both behaviours.
    let exited = wait_exit(&mut child).expect("staleness ends it once the app is gone");
    assert!(exited.success());
}

/// Demanded test 2: a job the daemon cannot trust is refused without walking, and
/// the refusal is reported per job (the app sees it in status-<n>.json) instead of
/// killing the session for everybody else.
#[test]
fn serve_mode_refuses_jobs_it_cannot_trust_without_walking() {
    let stage = tempfile::tempdir().expect("tempdir");
    let good = readable_fixture(1);
    let mut child = spawn_serve(stage.path(), 900, 900);
    touch_alive(stage.path());
    assert!(wait_ready(stage.path()));

    // Malformed schema, wrong version, a relative (unwalkable-by-contract) path, and
    // an absurdly oversized file — four refusals, each named by its exit code.
    fs::write(
        stage.path().join("job-bad.json"),
        "{\"version\":1,\"deadline_secs\":60,\"paths\":[],\"surprise\":true}",
    )
    .expect("write");
    fs::write(
        stage.path().join("job-ver.json"),
        "{\"version\":2,\"deadline_secs\":60,\"paths\":[]}",
    )
    .expect("write");
    fs::write(
        stage.path().join("job-rel.json"),
        "{\"version\":1,\"deadline_secs\":60,\"paths\":[\"relative/path\"]}",
    )
    .expect("write");
    fs::write(stage.path().join("job-huge.json"), vec![b'{'; 2 * 1024 * 1024]).expect("write");

    assert_eq!(wait_status(stage.path(), "bad")["exit"], 3, "unknown field");
    assert_eq!(wait_status(stage.path(), "ver")["exit"], 4, "wrong version");
    assert_eq!(wait_status(stage.path(), "rel")["exit"], 5, "relative path");
    assert_eq!(wait_status(stage.path(), "huge")["exit"], 6, "oversized job");
    for id in ["bad", "ver", "rel", "huge"] {
        assert!(
            !stage.path().join(format!("results-{id}.jsonl")).exists(),
            "{id}: a refused job must walk nothing"
        );
    }

    // And the daemon survives the abuse: the next honest job still completes.
    drop_job(stage.path(), "ok", &[good.path().to_str().expect("utf8")]);
    assert_eq!(wait_status(stage.path(), "ok")["exit"], 0);

    drop(stage);
    assert!(wait_exit(&mut child).is_some());
}
