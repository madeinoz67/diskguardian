//! The serve loop as a library entry, in-process.
//!
//! Windows has no sidecar: the app re-execs itself elevated and its pre-Tauri argv
//! branch calls `serve_standalone` directly (design §3.7.1, §3.7.3). The sidecar's own
//! lifecycle tests (`tests/serve.rs`) drive the binary from outside; this file drives
//! the exact entry the Windows path takes, from inside, so a divergence between the two
//! hosts of the same protocol would have to break one of the two suites to pass.
//!
//! Unprivileged, like every other admin test: the protocol is launcher-agnostic.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Bounds are generous multiples of the 200 ms poll interval: the property under test
/// is "it serves", not "it serves in exactly N ms".
const WAIT: Duration = Duration::from_secs(15);

fn drop_job(dir: &Path, id: &str, paths: &[&str]) {
    let quoted: Vec<String> = paths.iter().map(|p| format!("\"{p}\"")).collect();
    let body = format!(
        "{{\"version\":1,\"deadline_secs\":60,\"paths\":[{}]}}",
        quoted.join(",")
    );
    fs::write(dir.join(format!("job-{id}.json")), body).expect("write job");
}

#[test]
fn serve_standalone_serves_a_job_in_process() {
    let stage = tempfile::tempdir().expect("tempdir");
    let fixture = tempfile::tempdir().expect("tempdir");
    fs::write(fixture.path().join("f.bin"), vec![7u8; 64 * 1024]).expect("write fixture");
    drop_job(stage.path(), "1", &[fixture.path().to_str().expect("utf8")]);

    let dir = stage.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let code = diskguardian_admin::serve_standalone(&dir);
        let _ = tx.send(code);
    });

    assert!(
        wait_for(stage.path(), "daemon.ready", WAIT),
        "the serving instance must announce readiness"
    );
    assert!(
        wait_for(stage.path(), "status-1.json", WAIT),
        "the job must complete (status-1.json is the completion marker)"
    );
    let status: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(stage.path().join("status-1.json")).expect("status file"),
    )
    .expect("status is JSON");
    assert_eq!(status["exit"], 0, "the job ran: {status}");

    let results =
        fs::read_to_string(stage.path().join("results-1.jsonl")).expect("results file exists");
    let line: serde_json::Value =
        serde_json::from_str(results.lines().next().expect("one result line"))
            .expect("the result line is JSON");
    assert_eq!(line["status"], "ok");
    assert!(
        line["tree"]["nodes"][0]["total_bytes"]
            .as_u64()
            .expect("bytes")
            >= 64 * 1024
    );

    // Clean exit, the same signal the app sends its sidecar: the staging directory is
    // removed (a TempDir drop). The lib entry must return on that signal — bounded by
    // its poll interval, not by the 15-minute idle budget.
    drop(stage);
    let code = rx
        .recv_timeout(WAIT)
        .expect("serve_standalone must return when the session directory goes");
    assert_eq!(code, 0, "a clean session end exits 0");
    server.join().expect("the serving thread joined");
}

fn wait_for(dir: &Path, name: &str, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        if dir.join(name).exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}
