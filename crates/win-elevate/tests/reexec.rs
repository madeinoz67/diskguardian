//! The harness-less Windows re-exec smoke (design §3.7.5, optional arm).
//!
//! This is the only place anything exercises the real `ShellExecuteW` wrapper. It owns
//! its `main` because it must play two roles: the parent that calls `elevate_serve`,
//! and the elevated child the OS starts — a distinction no `#[test]` harness can
//! express. On GitHub's Windows runners the shell is already elevated, so the UAC
//! prompt cannot fire and the real path runs headlessly; it proves the wrapper and the
//! argv dispatch, NOT consent behaviour (that is Stage H, a human on real Windows).
//!
//! On a machine that is not elevated it SKIPS with a message rather than interrupting
//! whoever ran `cargo test` with a real consent dialog. **Pre-declared droppable**
//! (maintainer ruling 9, 2026-11-03): if this proves flaky in CI, delete this file and
//! the `[[test]]` stanza in Cargo.toml — nothing else depends on it.

#[cfg(not(windows))]
fn main() {
    // Off Windows there is no wrapper to exercise.
}

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const WAIT: Duration = Duration::from_secs(30);

    let args: Vec<String> = std::env::args().collect();
    // Child role: the parent elevated this same binary with the serve operand.
    if let Some(pos) = args.iter().position(|a| a == "--reexec-serve") {
        let dir = PathBuf::from(&args[pos + 1]);
        std::process::exit(diskguardian_admin::serve_standalone(&dir) as i32);
    }

    // Parent role.
    if !is_elevated() {
        println!("SKIP reexec smoke: not elevated (runs only where UAC cannot fire)");
        return;
    }

    let stage = tempfile::tempdir().expect("tempdir");
    let fixture = tempfile::tempdir().expect("tempdir");
    std::fs::write(fixture.path().join("smoke.bin"), vec![7u8; 64 * 1024]).expect("fixture");
    // The job is staged before the launch so the child can serve it the moment it is
    // up. Built with serde_json, not format!: a Windows temp path is full of
    // backslashes, which format! writes into the JSON unescaped — the first run of
    // this smoke served that job a BAD_JOB refusal (exit 3) for exactly that reason,
    // while proving everything else: elevation, the wrapper, the dispatch, readiness.
    let job = serde_json::json!({
        "version": 1,
        "deadline_secs": 60,
        "paths": [fixture.path().display().to_string()],
    })
    .to_string();
    std::fs::write(stage.path().join("job-1.json"), job).expect("job file");

    let exe = std::env::current_exe().expect("current exe");
    let parameters = format!("--reexec-serve \"{}\"", stage.path().display());
    let outcome =
        diskguardian_win_elevate::elevate_serve(&exe, &parameters).expect("elevate_serve runs");
    assert!(outcome.success, "elevation failed: {}", outcome.stderr);

    // daemon.ready then the completion marker; both bounded.
    let deadline = Instant::now() + WAIT;
    while !stage.path().join("daemon.ready").exists() {
        assert!(
            Instant::now() < deadline,
            "the elevated child never announced readiness"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let status_path = stage.path().join("status-1.json");
    let status_text = loop {
        if let Ok(text) = std::fs::read_to_string(&status_path) {
            break text;
        }
        assert!(
            Instant::now() < deadline,
            "status-1.json never appeared: the elevated child served nothing"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let status: serde_json::Value = serde_json::from_str(&status_text).expect("status is JSON");
    assert_eq!(
        status["exit"], 0,
        "the elevated child ran the job: {status}"
    );
    let results = std::fs::read_to_string(stage.path().join("results-1.jsonl"))
        .expect("the child wrote results");
    let line: serde_json::Value =
        serde_json::from_str(results.lines().next().expect("one line")).expect("line is JSON");
    assert_eq!(line["status"], "ok");

    // The child's exit is not asserted here: by design we hold no handle to it, and its
    // three exit budgets are pinned by crates/admin's own suite. Removing the staging
    // directory is the clean-exit signal; the child leaves on its own.
    drop(stage);
    drop(fixture);
    println!("reexec smoke: the real ShellExecuteW wrapper served a job round-trip");
}

#[cfg(windows)]
fn is_elevated() -> bool {
    // Verified present in windows-sys 0.61.2 (Shell32; deprecated by Microsoft but
    // functional, and a test-only probe). Skips the smoke anywhere a real UAC prompt
    // would otherwise interrupt the user.
    #[allow(deprecated)]
    unsafe {
        windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0
    }
}
