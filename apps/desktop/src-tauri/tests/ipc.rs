//! The IPC surface, driven the way the webview drives it.
//!
//! The commands that take more than one argument were only ever exercised by a human
//! clicking them, and one of them (`collect`) was reported as doing nothing at all. This
//! builds the same command surface the app exposes on Tauri's mock runtime and calls them
//! with the exact JSON the frontend sends.

use diskguardian_app::admin;
use diskguardian_app::configure;
use diskguardian_app::state::AppState;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::Manager;

fn build() -> tauri::App<MockRuntime> {
    configure(mock_builder())
        .build(mock_context(noop_assets()))
        .expect("mock app builds")
}

fn call(
    webview: &tauri::WebviewWindow<MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("url"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )?;
    Ok(response.deserialize::<serde_json::Value>().expect("json"))
}

/// Register a real scan of a temp directory, the way the scan command would.
fn seed_scan(app: &tauri::App<MockRuntime>) -> (String, u64) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("payload.bin"), vec![7u8; 128 * 1024]).expect("write");
    fs::create_dir(dir.path().join("sub")).expect("mkdir");
    fs::write(dir.path().join("sub/child.bin"), vec![7u8; 64 * 1024]).expect("write");

    let tree = diskguardian_core::scan(dir.path(), &Default::default()).expect("scan");
    let root = tree.root as u64;
    // Leak the tempdir so the paths in the tree stay valid for this test.
    std::mem::forget(dir);
    let scan_id = app.state::<AppState>().insert(tree);
    (scan_id, root)
}

fn child_named(
    webview: &tauri::WebviewWindow<MockRuntime>,
    scan_id: &str,
    root: u64,
    name: &str,
) -> u64 {
    let view = call(
        webview,
        "view",
        serde_json::json!({ "scanId": scan_id, "nodeId": root }),
    )
    .expect("view");
    view.as_array()
        .expect("array")
        .iter()
        .find(|node| node["name"] == name)
        .expect("child present")["node_id"]
        .as_u64()
        .expect("node id")
}

#[test]
fn collect_adds_a_node_and_reports_the_total() {
    let app = build();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");
    let (scan_id, root) = seed_scan(&app);

    // A direct child of the scan root, which is what a legend row is.
    let child = child_named(&webview, &scan_id, root, "sub");

    let result = call(
        &webview,
        "collect",
        serde_json::json!({ "scanId": scan_id, "nodeId": child, "add": true }),
    )
    .expect("collect must succeed");

    assert_eq!(result["count"], 1, "one item collected: {result}");
    let total = result["total"].as_u64().expect("total");
    assert!(
        total >= 64 * 1024,
        "total should include the nested file, got {total}"
    );
    assert_eq!(
        result["needs_confirmation"], true,
        "a direct child of the scan root always asks"
    );
}

#[test]
fn collecting_the_scanned_folder_itself_is_refused() {
    let app = build();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");
    let (scan_id, root) = seed_scan(&app);

    let result = call(
        &webview,
        "collect",
        serde_json::json!({ "scanId": scan_id, "nodeId": root, "add": true }),
    );

    assert!(result.is_err(), "the scan root must be refused");
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("cannot be collected"));
}

#[test]
fn collect_can_remove_again() {
    let app = build();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");
    let (scan_id, root) = seed_scan(&app);
    let child = child_named(&webview, &scan_id, root, "sub");

    let _ = call(
        &webview,
        "collect",
        serde_json::json!({ "scanId": scan_id, "nodeId": child, "add": true }),
    )
    .expect("add");
    let removed = call(
        &webview,
        "collect",
        serde_json::json!({ "scanId": scan_id, "nodeId": child, "add": false }),
    )
    .expect("remove");

    assert_eq!(removed["count"], 0);
    assert_eq!(removed["total"], 0);
}

#[test]
fn view_returns_a_pruned_tree_for_a_node() {
    let app = build();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");
    let (scan_id, root) = seed_scan(&app);

    let view = call(
        &webview,
        "view",
        serde_json::json!({ "scanId": scan_id, "nodeId": root }),
    )
    .expect("view must succeed");

    let nodes = view.as_array().expect("array");
    assert!(nodes.len() >= 3, "root, sub and two files: {view}");
    assert!(nodes[0]["bytes"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn an_unknown_scan_is_a_typed_error_not_a_panic() {
    let app = build();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");

    let result = call(
        &webview,
        "collect",
        serde_json::json!({ "scanId": "nope", "nodeId": 0, "add": true }),
    );

    assert!(result.is_err(), "must reject an unknown scan");
    assert!(
        result.unwrap_err().to_string().contains("unknown scan"),
        "the error must name the failure"
    );
}

#[test]
fn delete_strikes_the_node_through_and_shrinks_the_totals() {
    let app = build();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");
    let (scan_id, root) = seed_scan(&app);
    let child = child_named(&webview, &scan_id, root, "sub");

    let before = call(
        &webview,
        "view",
        serde_json::json!({ "scanId": scan_id, "nodeId": root }),
    )
    .expect("view");
    let before_bytes = before[0]["bytes"].as_u64().unwrap_or(0);

    call(
        &webview,
        "collect",
        serde_json::json!({ "scanId": scan_id, "nodeId": child, "add": true }),
    )
    .expect("collect");

    let report = call(
        &webview,
        "delete_collected",
        serde_json::json!({ "scanId": scan_id, "nodeId": root, "permanent": false, "confirmed": true }),
    )
    .expect("delete must succeed");

    assert_eq!(report["removed"], 1, "one item removed: {report}");
    assert_eq!(report["failed"], 0);
    assert_eq!(report["mode"], "trash");

    let after = report["view"].as_array().expect("view array");
    let struck = after
        .iter()
        .find(|node| node["name"] == "sub")
        .expect("the deleted folder stays listed so the user can see it");
    assert_eq!(
        struck["missing"], true,
        "and it is struck through: {report}"
    );
    // C22: worth nothing in the totals (the exact 65536 drop is asserted below) but
    // still drawn — the view gives a missing node a floor sliver so the strike-through
    // has something to draw on. This assertion was originally `== 0`, which the
    // landed floor (parent/200, minimum 1) makes unsatisfiable; it failed on main's
    // own commit before this branch touched anything, so it is pinned here as the
    // behaviour the ISA actually describes.
    let sliver = struck["bytes"].as_u64().expect("sliver bytes");
    assert!(sliver > 0, "the struck node stays visible: {report}");
    assert!(sliver < 4096, "a sliver, not a size: {sliver}");

    let after_bytes = report["bytes"].as_u64().unwrap_or(0);
    assert!(after_bytes < before_bytes, "the total must shrink");
    assert_eq!(
        before_bytes - after_bytes,
        65536,
        "exactly the deleted subtree, not the directory's metadata size"
    );
}

// ─── administrator measurement (R3, session model) ──────────────────────────────
//
// The privileged side is now one serving helper per app session (the principal's
// 2026-11-02 amendment): the first pass spends the one OS dialog and starts a daemon
// over the job/results files; later passes hand it more jobs with no second prompt.
// The launcher seam stands in for the dialog, so the command — session bookkeeping,
// job staging, verification, grafting, reporting — runs for real without a password.

use std::sync::atomic::{AtomicUsize, Ordering as CountOrdering};

/// A scan whose `locked/` directory this process cannot read: the shape an
/// administrator pass exists to fix.
fn seed_scan_with_locked(app: &tauri::App<MockRuntime>, name: &str) -> (String, u64, u64, PathBuf) {
    if name.is_empty() {
        panic!("the locked fixture needs a name");
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let locked = dir.path().join(name);
    fs::create_dir(&locked).expect("mkdir");
    fs::write(locked.join("secret.bin"), vec![7u8; 256 * 1024]).expect("write");
    fs::write(dir.path().join("plain.bin"), vec![7u8; 16 * 1024]).expect("write");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let tree = diskguardian_core::scan(dir.path(), &Default::default()).expect("scan");
    let locked_id = tree
        .nodes
        .iter()
        .position(|n| n.name == name)
        .expect("locked node present") as u64;
    let root = tree.root as u64;
    // Leak the fixture so the tree's paths stay valid for this test, as seed_scan does.
    std::mem::forget(dir);
    let scan_id = app.state::<AppState>().insert(tree);
    (scan_id, root, locked_id, locked)
}

/// A measured tree exactly as the helper produces one for `path`: a real scan of a
/// readable fixture, relabelled to the requested root.
fn canned_tree_for(path: &str) -> diskguardian_core::Tree {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("measured-0.bin"), vec![7u8; 512 * 1024]).expect("write");
    fs::write(dir.path().join("measured-1.bin"), vec![7u8; 512 * 1024]).expect("write");
    let mut tree = diskguardian_core::scan(dir.path(), &Default::default()).expect("scan");
    tree.scanned_root = PathBuf::from(path);
    tree
}

type Mangle = Arc<dyn Fn(&str, &str, &mut diskguardian_core::Tree) -> bool + Send + Sync>;

/// A stand-in for the dialog plus the serving helper: announces readiness, then
/// processes `job-<n>.json` files exactly as the real daemon does — same names, same
/// status-last completion marker. `mangle` sees (job stem, path, tree) and returns
/// whether that results line is delivered at all, so tests can corrupt or withhold
/// individual results.
///
/// Returns the launcher and a counter of how many times it was invoked — the
/// one-prompt-per-session property is asserted against that counter.
fn daemon_launcher(mangle: Mangle) -> (admin::Launch, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let launch: admin::Launch = Arc::new(move |dir: &Path| {
        counted.fetch_add(1, CountOrdering::SeqCst);
        let daemon_dir = dir.to_path_buf();
        let mangle = Arc::clone(&mangle);
        std::thread::spawn(move || {
            let _ = fs::write(daemon_dir.join("daemon.ready"), br#"{"version":1}"#);
            let mut done: Vec<String> = Vec::new();
            while daemon_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&daemon_dir) {
                    let mut jobs: Vec<String> = entries
                        .flatten()
                        .filter_map(|e| e.file_name().to_str().map(str::to_string))
                        .filter(|n| n.starts_with("job-") && n.ends_with(".json"))
                        .collect();
                    jobs.sort();
                    for job in jobs {
                        let stem = job
                            .strip_prefix("job-")
                            .and_then(|s| s.strip_suffix(".json"))
                            .unwrap_or("x")
                            .to_string();
                        if done.contains(&stem) {
                            continue;
                        }
                        done.push(stem.clone());
                        let Ok(text) = fs::read_to_string(daemon_dir.join(&job)) else {
                            continue;
                        };
                        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        let mut out = String::new();
                        let mut delivered = 0usize;
                        for path in doc["paths"].as_array().unwrap_or(&Vec::new()) {
                            let path = path.as_str().unwrap_or_default();
                            let mut tree = canned_tree_for(path);
                            if mangle(&stem, path, &mut tree) {
                                out.push_str(
                                    &serde_json::json!({
                                        "version": 1,
                                        "path": path,
                                        "status": "ok",
                                        "tree": tree,
                                    })
                                    .to_string(),
                                );
                                out.push('\n');
                                delivered += 1;
                            }
                        }
                        let _ = fs::write(daemon_dir.join(format!("results-{stem}.jsonl")), out);
                        let _ = fs::write(
                            daemon_dir.join(format!("status-{stem}.json")),
                            serde_json::json!({
                                "version": 1,
                                "exit": 0,
                                "completed": delivered,
                                "deadline_hit": false,
                            })
                            .to_string(),
                        );
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        Ok(admin::LaunchOutcome {
            success: true,
            stderr: String::new(),
        })
    });
    (launch, calls)
}

fn honest_daemon() -> (admin::Launch, Arc<AtomicUsize>) {
    daemon_launcher(Arc::new(|_, _, _| true))
}

fn inject_launcher(app: &tauri::App<MockRuntime>, launch: admin::Launch) {
    *app.state::<AppState>()
        .admin_launch
        .lock()
        .expect("launcher seam") = Some(launch);
}

fn webview(app: &tauri::App<MockRuntime>) -> tauri::WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "main", Default::default())
        .build()
        .expect("webview")
}

#[test]
fn scan_restricted_grafts_measured_areas_and_returns_a_view() {
    let app = build();
    let webview = webview(&app);
    let (scan_id, root, _locked, _path) = seed_scan_with_locked(&app, "locked");
    let (launch, calls) = honest_daemon();
    inject_launcher(&app, launch);

    let before = call(
        &webview,
        "view",
        serde_json::json!({ "scanId": scan_id, "nodeId": root }),
    )
    .expect("view");
    let before_bytes = before[0]["bytes"].as_u64().expect("bytes");
    let restricted = before
        .as_array()
        .expect("array")
        .iter()
        .find(|n| n["name"] == "locked")
        .expect("locked listed before the pass");
    assert_eq!(restricted["kind"], "restricted");

    let report = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    )
    .expect("scan_restricted must succeed");

    assert_eq!(report["patched"], 1, "one area grafted: {report}");
    assert_eq!(
        report["still_restricted"], 0,
        "nothing stays restricted: {report}"
    );
    let after_bytes = report["bytes"].as_u64().expect("bytes");
    assert!(
        after_bytes > before_bytes + 512 * 1024,
        "the measured payload must land in the totals: {before_bytes} -> {after_bytes}"
    );

    let view = report["view"].as_array().expect("view");
    assert_eq!(view[0]["partial"], false, "the root is fully measured now");
    let measured = view
        .iter()
        .find(|n| n["name"] == "locked")
        .expect("locked still listed");
    assert_eq!(measured["kind"], "dir", "the petal is measured: {report}");
    assert_eq!(
        calls.load(CountOrdering::SeqCst),
        1,
        "one pass, one dialog spent"
    );
}

/// The amendment's core property: a second pass in the same app run reuses the
/// daemon. Pass one here also demonstrates a withheld result (one path's line never
/// delivered), which is what leaves an area for pass two to measure.
#[test]
fn a_second_pass_reuses_the_daemon_without_a_second_prompt() {
    let app = build();
    let webview = webview(&app);
    let (scan_id, root, _a, _b) = seed_two_locked(&app);
    let (launch, calls) = daemon_launcher(Arc::new(|stem, path, _tree| {
        // Pass one delivers only the first area; pass two delivers the rest.
        !(stem == "1" && path.ends_with("locked2"))
    }));
    inject_launcher(&app, launch);

    let first = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    )
    .expect("first pass");
    assert_eq!(first["patched"], 1, "{first}");
    assert_eq!(
        first["still_restricted"], 1,
        "one area was withheld: {first}"
    );

    let second = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    )
    .expect("second pass");
    assert_eq!(second["patched"], 1, "{second}");
    assert_eq!(second["still_restricted"], 0, "{second}");

    assert_eq!(
        calls.load(CountOrdering::SeqCst),
        1,
        "two passes, still exactly one dialog"
    );
    let _ = root;
}

/// One fixture, two restricted areas — for the multi-pass test.
fn seed_two_locked(app: &tauri::App<MockRuntime>) -> (String, u64, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("locked");
    let b = dir.path().join("locked2");
    for (path, size) in [(&a, 256 * 1024), (&b, 128 * 1024)] {
        fs::create_dir(path).expect("mkdir");
        fs::write(path.join("secret.bin"), vec![7u8; size]).expect("write");
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).expect("chmod");
    }
    let tree = diskguardian_core::scan(dir.path(), &Default::default()).expect("scan");
    let root = tree.root as u64;
    std::mem::forget(dir);
    let scan_id = app.state::<AppState>().insert(tree);
    (scan_id, root, a, b)
}

#[test]
fn scan_restricted_rejects_an_unknown_scan() {
    let app = build();
    let webview = webview(&app);
    let (launch, _calls) = honest_daemon();
    inject_launcher(&app, launch);

    let result = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": "nope" }),
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("unknown scan"));
}

#[test]
fn scan_restricted_with_nothing_restricted_is_an_error() {
    let app = build();
    let webview = webview(&app);
    let (scan_id, _root) = seed_scan(&app);
    let (launch, _calls) = honest_daemon();
    inject_launcher(&app, launch);

    let result = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    );
    assert!(result.is_err(), "there is nothing to measure");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("nothing to measure"),
        "the error must say why"
    );
}

#[test]
fn a_declined_dialog_is_a_typed_refusal_not_a_crash() {
    let app = build();
    let webview = webview(&app);
    let (scan_id, _root, _locked, _path) = seed_scan_with_locked(&app, "locked");
    inject_launcher(
        &app,
        Arc::new(|_dir: &Path| {
            Ok(admin::LaunchOutcome {
                success: false,
                stderr: "31:83: execution error: User canceled. (-128)".to_string(),
            })
        }),
    );

    let result = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    );
    assert!(result.is_err(), "a cancelled dialog must be an error");
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("declined"),
        "the refusal must be typed, got: {message}"
    );
    assert!(
        message.contains("unmeasured"),
        "and it must say the tree is untouched: {message}"
    );
}

#[test]
fn a_failed_helper_reports_its_stderr() {
    let app = build();
    let webview = webview(&app);
    let (scan_id, _root, _locked, _path) = seed_scan_with_locked(&app, "locked");
    inject_launcher(
        &app,
        Arc::new(|_dir: &Path| {
            Ok(admin::LaunchOutcome {
                success: false,
                stderr: "helper exploded".to_string(),
            })
        }),
    );

    let result = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    );
    let message = result
        .expect_err("a failed helper must be an error")
        .to_string();
    assert!(
        message.contains("helper exploded"),
        "the stderr must surface: {message}"
    );
}

/// A launcher that reports approval but never brings a daemon up: the command must
/// say the helper did not start, not hang until the pass budget expires.
#[test]
fn a_helper_that_never_becomes_ready_is_a_named_failure() {
    std::env::set_var("DISKGUARDIAN_TEST_READY_WAIT_MS", "200");
    let app = build();
    let webview = webview(&app);
    let (scan_id, _root, _locked, _path) = seed_scan_with_locked(&app, "locked");
    inject_launcher(
        &app,
        Arc::new(|_dir: &Path| {
            Ok(admin::LaunchOutcome {
                success: true,
                stderr: String::new(),
            })
        }),
    );

    let result = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    );
    let message = result
        .expect_err("an approved-but-absent helper must fail")
        .to_string();
    assert!(
        message.contains("did not start"),
        "the failure must name the mechanism: {message}"
    );
}

#[test]
fn a_result_whose_root_does_not_match_is_not_grafted() {
    let app = build();
    let webview = webview(&app);
    let (scan_id, root, _locked, _path) = seed_scan_with_locked(&app, "locked");
    let (launch, _calls) = daemon_launcher(Arc::new(|_stem, _path, tree| {
        // A tampered or misdelivered result: measured somewhere else.
        tree.scanned_root = PathBuf::from("/elsewhere/entirely");
        true
    }));
    inject_launcher(&app, launch);

    let before = call(
        &webview,
        "view",
        serde_json::json!({ "scanId": scan_id, "nodeId": root }),
    )
    .expect("view");
    let before_bytes = before[0]["bytes"].as_u64().expect("bytes");

    let report = call(
        &webview,
        "scan_restricted",
        serde_json::json!({ "scanId": scan_id }),
    )
    .expect("the command still completes");

    assert_eq!(report["patched"], 0, "nothing may be grafted from it");
    assert_eq!(
        report["still_restricted"], 1,
        "the area is still restricted"
    );
    assert_eq!(
        report["bytes"].as_u64().expect("bytes"),
        before_bytes,
        "the tree must be untouched"
    );
}

/// T1: the injection class this design exists to kill. Hostile names on the disk —
/// a folder named to look like AppleScript or shell syntax — must travel only inside
/// the JSON job file, and must never appear in the dialog's source, the child argv,
/// or anywhere else a shell would read them.
#[test]
fn hostile_names_travel_in_the_job_file_only() {
    let hostile = [
        "x\"; do shell script \"curl bad.example\"; echo \"",
        "back`ticks`",
        "$(injection)",
        "semi;colon",
        "new\nline",
    ];

    for name in hostile {
        let app = build();
        let webview = webview(&app);
        let (scan_id, _root, _locked, locked_path) = seed_scan_with_locked(&app, name);

        // Capture the job files the command staged, through the daemon seam — the
        // session directory is removed when the app state drops, so everything about
        // them must be observed while the pass is live.
        let captured: Arc<Mutex<Vec<(PathBuf, String, u32)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captured);
        inject_launcher(
            &app,
            Arc::new(move |dir: &Path| {
                let watch = dir.to_path_buf();
                let sink = Arc::clone(&sink);
                std::thread::spawn(move || {
                    let _ = fs::write(watch.join("daemon.ready"), br#"{"version":1}"#);
                    let mut seen: Vec<String> = Vec::new();
                    while watch.is_dir() {
                        if let Ok(entries) = fs::read_dir(&watch) {
                            let mut jobs: Vec<String> = entries
                                .flatten()
                                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                                .filter(|n| n.starts_with("job-") && n.ends_with(".json"))
                                .collect();
                            jobs.sort();
                            for job in jobs {
                                if seen.contains(&job) {
                                    continue;
                                }
                                seen.push(job.clone());
                                let path = watch.join(&job);
                                let mode = fs::metadata(&path)
                                    .map(|m| m.permissions().mode() & 0o777)
                                    .unwrap_or(0);
                                let body = fs::read_to_string(&path).unwrap_or_default();
                                sink.lock().expect("capture").push((path, body, mode));
                                // No results: the pass completes having grafted nothing.
                                let stem = job
                                    .strip_prefix("job-")
                                    .and_then(|s| s.strip_suffix(".json"))
                                    .unwrap_or("x");
                                let _ = fs::write(
                                    watch.join(format!("status-{stem}.json")),
                                    r#"{"version":1,"exit":0,"completed":0,"deadline_hit":false}"#,
                                );
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                });
                Ok(admin::LaunchOutcome {
                    success: true,
                    stderr: String::new(),
                })
            }),
        );

        let report = call(
            &webview,
            "scan_restricted",
            serde_json::json!({ "scanId": scan_id }),
        )
        .expect("the pass runs and grafts nothing (no results were written)");
        assert_eq!(report["patched"], 0);

        let jobs = captured.lock().expect("capture").clone();
        assert_eq!(jobs.len(), 1, "one job staged: {jobs:?}");
        let (job_path, job_body, job_mode) = jobs.first().expect("captured").clone();
        let session_dir = job_path
            .parent()
            .expect("job lives in the session dir")
            .to_path_buf();

        // 1. The job round-trips the hostile name unchanged: the helper must be able
        //    to walk the real path.
        let doc: serde_json::Value = serde_json::from_str(&job_body).expect("job is JSON");
        let paths: Vec<String> = doc["paths"]
            .as_array()
            .expect("paths")
            .iter()
            .map(|p| p.as_str().expect("string").to_string())
            .collect();
        assert_eq!(
            paths,
            vec![locked_path.display().to_string()],
            "the job must name exactly the restricted root, hostile bytes and all"
        );

        // 2. The job file itself is private to this user.
        assert_eq!(job_mode, 0o600, "the job file is 0600: {job_mode:o}");

        // 3. The dialog source and the child argv contain none of it: byte-identical
        //    to the fixed template, whatever the disk happens to be called. Only two
        //    app-generated strings move: the helper and the staging directory.
        let helper = "/Applications/DiskGuardian.app/Contents/MacOS/diskguardian-admin";
        let source =
            admin::applescript_source(helper, &session_dir.display().to_string()).expect("source");
        assert_eq!(
            source,
            format!(
                "do shell script \"{helper} --serve {} > /dev/null 2>&1 &\" \
                 with administrator privileges",
                session_dir.display()
            ),
            "the template is fixed: only the two app-generated paths move"
        );
        // The template's own quotes are the only quotes that may appear; everything
        // else hostile — including the fixture's whole name — must be absent. (The
        // byte-identity assert above already pins the structure; this makes a failure
        // readable when it ever bites.)
        let disarmed = source.replace('"', "");
        for fragment in [";", "`", "$", "curl", "\n", name] {
            assert!(
                !disarmed.contains(fragment),
                "the dialog source must not contain {fragment:?}: {source}"
            );
        }
        assert_eq!(
            admin::osascript_args(&source),
            vec!["-e".to_string(), source.clone()],
            "the child argv is the template and nothing else"
        );

        // Restore readability so the leaked fixture directory is not a surprise.
        let _ = fs::set_permissions(&locked_path, fs::Permissions::from_mode(0o755));
    }
}

#[test]
fn template_inputs_outside_the_asserted_charset_are_refused() {
    for bad in [
        "a;b",
        "a b (paren)",
        "quote\"in",
        "back\\slash",
        "tab\tchar",
        "",
    ] {
        assert!(
            admin::applescript_source("/safe/helper", bad).is_err(),
            "{bad:?} must be refused before any dialog is built"
        );
        assert!(
            admin::applescript_source(bad, "/safe/staging-dir").is_err(),
            "{bad:?} as the helper path must be refused too"
        );
    }
    // The full legal set still interpolates.
    assert!(admin::applescript_source(
        "/usr/local/bin/diskguardian-admin-x9_v2",
        "/var/folders/Tz/abc.d/staging-1"
    )
    .is_ok());
}

// ─── duplicates ───────────────────────────────────────────────────────────────────────
//
// The engine's own tests prove the hashing; these prove the command the panel actually
// calls: the JSON the frontend sends, the shape it gets back, and the two honesty
// properties the UI is not allowed to lose — the caveat rides every group, and a pass
// that ran out of time says so instead of reporting "no duplicates".

/// A scan of a directory holding two content-identical files in different folders plus a
/// same-size file that differs in the middle, so the digest stage has real work to do.
fn seed_duplicates() -> (tauri::App<MockRuntime>, String) {
    let app = build();
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("a")).expect("mkdir a");
    fs::create_dir_all(dir.path().join("b")).expect("mkdir b");

    let payload = vec![0xA5u8; 256 * 1024];
    fs::write(dir.path().join("a/report.pdf"), &payload).expect("write a");
    fs::write(dir.path().join("b/report copy.pdf"), &payload).expect("write b");

    // Same allocation, different content: admitted to the size group, refused by the hash.
    let mut different = payload.clone();
    different[100] = 0x5A;
    fs::write(dir.path().join("b/other.pdf"), &different).expect("write c");

    let tree = diskguardian_core::scan(dir.path(), &Default::default()).expect("scan");
    // Leak the tempdir so the paths in the tree stay valid for this test.
    std::mem::forget(dir);
    let scan_id = app.state::<AppState>().insert(tree);
    (app, scan_id)
}

#[test]
fn duplicates_finds_the_identical_pair_and_carries_the_caveat() {
    let (app, scan_id) = seed_duplicates();
    let webview = webview(&app);

    let report = call(
        &webview,
        "duplicates",
        serde_json::json!({ "scanId": scan_id, "minBytes": 1024u64, "budgetMs": 30_000u64 }),
    )
    .expect("duplicates");

    let groups = report["groups"].as_array().expect("groups");
    assert_eq!(
        groups.len(),
        1,
        "one pair; the same-size file with different bytes must not join it: {report}"
    );

    let group = &groups[0];
    assert_eq!(
        group["members"].as_array().expect("members").len(),
        2,
        "both copies are in the group"
    );

    // The caveat is the engine's own sentence, carried per group. The panel shows these
    // exact words, so the command must not paraphrase or drop them.
    assert_eq!(
        group["caveat"].as_str(),
        Some(diskguardian_core::CONTENT_IDENTICAL_CAVEAT)
    );

    // With no clones in play, "would be freed" is one member's allocation.
    assert_eq!(group["wasted_bytes"], group["bytes_each"]);
    assert_eq!(report["partial"], serde_json::Value::Bool(false));
}

#[test]
fn duplicates_with_nothing_above_the_floor_is_an_empty_result_not_an_error() {
    let (app, scan_id) = seed_duplicates();
    let webview = webview(&app);

    // 1 GiB floor: the pair exists but is far below it, so nothing is even opened.
    let report = call(
        &webview,
        "duplicates",
        serde_json::json!({ "scanId": scan_id, "minBytes": 1_073_741_824u64 }),
    )
    .expect("duplicates");

    assert_eq!(report["groups"].as_array().expect("groups").len(), 0);
    assert_eq!(report["files_hashed"], serde_json::Value::from(0));
    assert_eq!(report["partial"], serde_json::Value::Bool(false));
}

#[test]
fn a_spent_budget_reports_partial_rather_than_no_duplicates() {
    let (app, scan_id) = seed_duplicates();
    let webview = webview(&app);

    // Zero budget: the pass is over before it reads its first candidate. The honest
    // answer is "we did not finish", never "there are none".
    let report = call(
        &webview,
        "duplicates",
        serde_json::json!({ "scanId": scan_id, "minBytes": 1024u64, "budgetMs": 0u64 }),
    )
    .expect("duplicates");

    assert_eq!(report["partial"], serde_json::Value::Bool(true));
    assert_eq!(report["groups"].as_array().expect("groups").len(), 0);
}

#[test]
fn duplicates_of_an_unknown_scan_is_a_typed_error() {
    let app = build();
    let webview = webview(&app);

    let result = call(
        &webview,
        "duplicates",
        serde_json::json!({ "scanId": "nope", "minBytes": 1024u64 }),
    );

    assert!(result.is_err(), "must reject an unknown scan");
    assert!(
        result.unwrap_err().to_string().contains("unknown scan"),
        "the error must name the failure"
    );
}

/// Hard links are one inode with several names: deleting one name frees nothing, so no
/// name of a shared inode may ever be proposed. Unix-only because inode identity comes
/// from `(dev, ino)`, which the engine cannot read on Windows — see ISA C3.
///
/// The shared inode is given the same allocation as a genuine duplicate pair, because a
/// hard link that is alone in its size bucket is settled for free and never reaches the
/// `nlink` check. This fixture makes both stages speak: the pair must be found, the
/// linked name must be refused.
#[cfg(unix)]
#[test]
fn duplicates_never_proposes_one_name_of_a_shared_inode() {
    let app = build();
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("a")).expect("mkdir a");
    fs::create_dir_all(dir.path().join("b")).expect("mkdir b");

    fs::write(dir.path().join("original.bin"), vec![3u8; 256 * 1024]).expect("write");
    fs::hard_link(
        dir.path().join("original.bin"),
        dir.path().join("second name.bin"),
    )
    .expect("hard link");

    let payload = vec![0xA5u8; 256 * 1024];
    fs::write(dir.path().join("a/report.pdf"), &payload).expect("write a");
    fs::write(dir.path().join("b/report copy.pdf"), &payload).expect("write b");

    let tree = diskguardian_core::scan(dir.path(), &Default::default()).expect("scan");
    std::mem::forget(dir);
    let scan_id = app.state::<AppState>().insert(tree);

    let webview = webview(&app);
    let report = call(
        &webview,
        "duplicates",
        serde_json::json!({ "scanId": scan_id, "minBytes": 1024u64, "budgetMs": 30_000u64 }),
    )
    .expect("duplicates");

    let groups = report["groups"].as_array().expect("groups");
    assert_eq!(
        groups.len(),
        1,
        "the genuine pair is found and the shared inode is not: {report}"
    );
    let members = groups[0]["members"].as_array().expect("members");
    assert_eq!(members.len(), 2);
    for member in members {
        let path = member["path"].as_str().unwrap_or_default();
        assert!(
            !path.contains("original.bin"),
            "a name of a shared inode must never be proposed: {path}"
        );
    }
    // The name that carries the bytes is refused at candidate time, and counted so the
    // exclusion is visible rather than silent.
    assert_eq!(
        report["excluded_links"].as_u64(),
        Some(1),
        "exactly the linked name is excluded: {report}"
    );
}

// ─── local snapshots ──────────────────────────────────────────────────────────────────
//
// The purge command is destructive and, on a real machine, would need administrator rights
// it deliberately does not have. Nothing here runs `tmutil` for real: the module's runner
// seam stands in for it, so these tests prove the command's answers without a single
// snapshot being touched.

#[cfg(target_os = "macos")]
#[test]
fn a_refused_purge_is_reported_in_words_and_deletes_nothing() {
    use diskguardian_app::snapshots::{set_runner_for_tests, TmOutput};

    const ONE: &str = "Snapshots for disk /:\ncom.apple.TimeMachine.2026-11-01-085943.local\n";

    fn refusing(args: &[String]) -> TmOutput {
        if args.first().map(String::as_str) == Some("listlocalsnapshots") {
            TmOutput {
                ok: true,
                stdout: ONE.to_string(),
                stderr: String::new(),
            }
        } else {
            TmOutput {
                ok: false,
                stdout: String::new(),
                stderr: "tmutil: deletelocalsnapshots requires root privileges.".to_string(),
            }
        }
    }

    fn succeeding(args: &[String]) -> TmOutput {
        if args.first().map(String::as_str) == Some("listlocalsnapshots") {
            TmOutput {
                ok: true,
                stdout: "Snapshots for disk /:\n".to_string(),
                stderr: String::new(),
            }
        } else {
            TmOutput {
                ok: true,
                stdout: String::new(),
                stderr: String::new(),
            }
        }
    }

    let app = build();
    let webview = webview(&app);

    set_runner_for_tests(Some(refusing));
    let listed = call(&webview, "local_snapshots", serde_json::json!({})).expect("list");
    assert_eq!(
        listed["snapshots"].as_array().expect("snapshots").len(),
        1,
        "the listing must survive to the frontend: {listed}"
    );
    assert_eq!(listed["supported"], serde_json::Value::Bool(true));

    let report = call(
        &webview,
        "purge_local_snapshots",
        serde_json::json!({ "confirmed": true }),
    )
    .expect("purge");

    assert_eq!(report["purged"], serde_json::Value::Bool(false));
    assert_eq!(
        report["refused"],
        serde_json::Value::Bool(true),
        "a root-rights refusal is its own outcome, not a generic failure: {report}"
    );
    let message = report["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("administrator rights"),
        "the message must name the cause: {message}"
    );
    assert!(
        message.contains("System Settings"),
        "the message must say what the user can do instead: {message}"
    );
    // A refused purge deleted nothing, so the snapshot is still listed.
    assert_eq!(report["remaining"].as_array().expect("remaining").len(), 1);

    // The other outcome: when tmutil does succeed, the report says so and the list is empty.
    set_runner_for_tests(Some(succeeding));
    let done = call(
        &webview,
        "purge_local_snapshots",
        serde_json::json!({ "confirmed": true }),
    )
    .expect("purge");
    assert_eq!(done["purged"], serde_json::Value::Bool(true));
    assert_eq!(done["refused"], serde_json::Value::Bool(false));
    assert_eq!(done["remaining"].as_array().expect("remaining").len(), 0);

    set_runner_for_tests(None);
}

#[cfg(target_os = "macos")]
#[test]
fn a_purge_without_confirmation_is_refused_before_anything_runs() {
    let app = build();
    let webview = webview(&app);

    let result = call(
        &webview,
        "purge_local_snapshots",
        serde_json::json!({ "confirmed": false }),
    );

    assert!(result.is_err(), "an unconfirmed purge must not proceed");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("explicit confirmation"),
        "the error must say what is missing"
    );
}

#[test]
fn cancelling_with_nothing_running_is_a_quiet_no_op() {
    // The frontend calls this before every scan, so the first call of an app run finds no
    // walk to cancel. It must answer, not error, and certainly not panic.
    let app = build();
    let webview = webview(&app);

    let first = call(&webview, "cancel_scan", serde_json::json!({}));
    let second = call(&webview, "cancel_scan", serde_json::json!({}));

    assert!(first.is_ok(), "the first cancel must not fail: {first:?}");
    assert!(second.is_ok(), "and it stays idempotent: {second:?}");
}

// ─── favourite folders (R8) ───────────────────────────────────────────────────────────
//
// Favourites are user data that must survive relaunch, so the store is a JSON file the
// Rust side owns and the webview can only reach through commands. The seam
// (`Favourites::set_path`) points one app's store at a temp directory before first use —
// the `admin_launch` pattern — which is also what makes the relaunch test possible: a
// second app on the same store file is a relaunch in miniature.

mod favourites {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    /// A private directory for one test's favourites store. Tests run in parallel, so
    /// every test gets its own; leaked the way scan fixtures are, because the store file
    /// must outlive the app that wrote it.
    fn store_dir() -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        std::mem::forget(dir);
        path
    }

    /// Point one app's favourites store at `dir` before its first use.
    fn set_store(app: &tauri::App<MockRuntime>, dir: &Path) {
        app.state::<AppState>()
            .favourites
            .lock()
            .expect("favourites slot")
            .set_path(dir.join("favourites.json"));
    }

    fn star_node(
        webview: &tauri::WebviewWindow<MockRuntime>,
        scan_id: &str,
        node_id: u64,
        add: bool,
    ) -> Result<serde_json::Value, serde_json::Value> {
        call(
            webview,
            "favourite_node",
            serde_json::json!({ "scanId": scan_id, "nodeId": node_id, "add": add }),
        )
    }

    fn set_favourite(
        webview: &tauri::WebviewWindow<MockRuntime>,
        path: &str,
        add: bool,
    ) -> Result<serde_json::Value, serde_json::Value> {
        call(
            webview,
            "set_favourite",
            serde_json::json!({ "path": path, "add": add }),
        )
    }

    #[test]
    fn starring_a_folder_persists_across_relaunch() {
        let dir = store_dir();
        let app = build();
        set_store(&app, &dir);
        let view = webview(&app);
        let (scan_id, root) = seed_scan(&app);
        let child = child_named(&view, &scan_id, root, "sub");

        let list = star_node(&view, &scan_id, child, true).expect("star");
        assert_eq!(list.as_array().expect("list").len(), 1);

        // Tear the app down completely, then bring a fresh one up on the same store file.
        drop(view);
        drop(app);
        let relaunched = build();
        set_store(&relaunched, &dir);
        let relaunched_view = webview(&relaunched);

        let after =
            call(&relaunched_view, "favourites", serde_json::json!({})).expect("favourites");
        let entries = after["favourites"].as_array().expect("entries");
        assert_eq!(
            entries.len(),
            1,
            "the favourite survived the relaunch: {after}"
        );
        assert_eq!(after["notice"], serde_json::Value::Null);
        assert!(
            entries[0]["path"]
                .as_str()
                .unwrap_or_default()
                .ends_with("sub"),
            "it is the starred folder: {after}"
        );
        assert_eq!(entries[0]["exists"], serde_json::Value::Bool(true));
    }

    #[test]
    fn starring_twice_is_a_no_op_never_a_duplicate_row() {
        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);
        let (scan_id, root) = seed_scan(&app);
        let child = child_named(&webview, &scan_id, root, "sub");

        star_node(&webview, &scan_id, child, true).expect("first star");
        let second =
            star_node(&webview, &scan_id, child, true).expect("a second star is a no-op answer");
        assert_eq!(
            second.as_array().expect("list").len(),
            1,
            "one entry, never two: {second}"
        );

        let report = call(&webview, "favourites", serde_json::json!({})).expect("favourites");
        assert_eq!(
            report["favourites"].as_array().expect("entries").len(),
            1,
            "the store agrees: {report}"
        );
    }

    /// A folder reachable through two spellings: a real directory and a symlink to it.
    /// Built rather than borrowed from `/tmp`, because whether `/tmp` is itself a symlink
    /// is a platform property (it is on macOS, it is not on Linux), and this test must
    /// fail for the right reason or not at all.
    #[cfg(unix)]
    fn symlinked_pair() -> (PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real-folder");
        fs::create_dir(&real).expect("mkdir");
        symlink("real-folder", dir.path().join("alias")).expect("symlink");
        let base = dir.path().to_path_buf();
        std::mem::forget(dir);
        (base.join("alias"), real)
    }

    #[cfg(unix)]
    #[test]
    fn symlink_spellings_collapse_to_one_entry() {
        let (alias, real) = symlinked_pair();
        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);

        set_favourite(&webview, &alias.display().to_string(), true)
            .expect("star the alias spelling");
        set_favourite(&webview, &real.display().to_string(), true).expect("star the real spelling");

        let report = call(&webview, "favourites", serde_json::json!({})).expect("favourites");
        let entries = report["favourites"].as_array().expect("entries");
        assert_eq!(
            entries.len(),
            1,
            "two spellings, one folder, one row: {report}"
        );
        let expected = fs::canonicalize(&real).expect("canonical");
        assert_eq!(
            entries[0]["path"].as_str(),
            Some(expected.display().to_string().as_str()),
            "the stored key is the canonical spelling"
        );
    }

    #[test]
    fn a_missing_favourite_is_reported_not_dropped() {
        let holder = tempfile::tempdir().expect("tempdir");
        let target = holder.path().join("gone-soon");
        fs::create_dir(&target).expect("mkdir");

        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);
        set_favourite(&webview, &target.display().to_string(), true).expect("star");

        fs::remove_dir_all(&target).expect("remove the folder out from under the app");

        let report = call(&webview, "favourites", serde_json::json!({})).expect("favourites");
        let entries = report["favourites"].as_array().expect("entries");
        assert_eq!(
            entries.len(),
            1,
            "a favourite that has gone stays listed, never dropped: {report}"
        );
        assert_eq!(
            entries[0]["exists"],
            serde_json::Value::Bool(false),
            "and its detail line can say why"
        );
    }

    #[test]
    fn marks_resolve_children_and_self() {
        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);
        let (scan_id, root) = seed_scan(&app);
        let child = child_named(&webview, &scan_id, root, "sub");
        star_node(&webview, &scan_id, child, true).expect("star the child");

        let at_parent = call(
            &webview,
            "favourite_marks",
            serde_json::json!({ "scanId": scan_id, "nodeId": root }),
        )
        .expect("marks at the parent");
        let children = at_parent["children"].as_array().expect("children");
        assert_eq!(children.len(), 1, "the starred child is named: {at_parent}");
        assert_eq!(children[0].as_u64(), Some(child));
        assert_eq!(at_parent["self_"], serde_json::Value::Bool(false));

        let at_child = call(
            &webview,
            "favourite_marks",
            serde_json::json!({ "scanId": scan_id, "nodeId": child }),
        )
        .expect("marks at the child");
        assert_eq!(
            at_child["self_"],
            serde_json::Value::Bool(true),
            "the folder itself is marked: {at_child}"
        );
        assert_eq!(
            at_child["children"].as_array().expect("children").len(),
            0,
            "its only child is a file"
        );
    }

    #[test]
    fn files_and_missing_nodes_cannot_be_favourited() {
        let holder = tempfile::tempdir().expect("tempdir");
        fs::create_dir(holder.path().join("sub")).expect("mkdir");
        fs::write(holder.path().join("sub/child.bin"), vec![7u8; 64 * 1024]).expect("write");
        fs::write(holder.path().join("loose.bin"), vec![7u8; 32 * 1024]).expect("write");

        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);
        let tree = diskguardian_core::scan(holder.path(), &Default::default()).expect("scan");
        let root = tree.root as u64;
        let scan_id = app.state::<AppState>().insert(tree);

        let file_id = child_named(&webview, &scan_id, root, "loose.bin");
        let refusal = star_node(&webview, &scan_id, file_id, true)
            .expect_err("a file must be refused")
            .to_string();
        assert!(
            refusal.contains("only folders"),
            "the error must say what is wrong: {refusal}"
        );

        // The folder then vanishes outside the app; revalidate flags it missing, and a
        // missing node is refused with its own words rather than starring a ghost.
        let sub_id = child_named(&webview, &scan_id, root, "sub");
        fs::remove_dir_all(holder.path().join("sub")).expect("remove");
        call(
            &webview,
            "revalidate",
            serde_json::json!({ "scanId": scan_id, "nodeId": root }),
        )
        .expect("revalidate");
        let refusal = star_node(&webview, &scan_id, sub_id, true)
            .expect_err("a missing node must be refused")
            .to_string();
        assert!(
            refusal.contains("no longer on disk"),
            "the error must say what is wrong: {refusal}"
        );

        let report = call(&webview, "favourites", serde_json::json!({})).expect("favourites");
        assert_eq!(
            report["favourites"].as_array().expect("entries").len(),
            0,
            "neither refusal wrote anything: {report}"
        );
    }

    #[test]
    fn set_favourite_refuses_relative_and_dotdot_paths() {
        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);

        for bad in ["relative/path", "/tmp/../etc", "", "~/Documents"] {
            let refusal = set_favourite(&webview, bad, true)
                .expect_err("a malformed path must be refused before anything is written")
                .to_string();
            assert!(
                refusal.contains("absolute path"),
                "{bad:?} refused with words that name the rule: {refusal}"
            );
        }

        let report = call(&webview, "favourites", serde_json::json!({})).expect("favourites");
        assert_eq!(
            report["favourites"].as_array().expect("entries").len(),
            0,
            "nothing was written by any refusal: {report}"
        );
    }

    #[test]
    fn a_corrupt_store_is_reported_and_reset() {
        let dir = store_dir();
        fs::write(dir.join("favourites.json"), b"{ this is not json").expect("garbage");

        let app = build();
        set_store(&app, &dir);
        let view = webview(&app);

        let report =
            call(&view, "favourites", serde_json::json!({})).expect("the report still answers");
        assert_eq!(
            report["favourites"].as_array().expect("entries").len(),
            0,
            "reset to empty"
        );
        let notice = report["notice"].as_str().unwrap_or_default();
        assert!(
            !notice.is_empty(),
            "corruption is visible text, never a silent empty list: {report}"
        );
        assert!(
            notice.contains("could not be read"),
            "the notice names what happened: {notice}"
        );

        let preserved: Vec<String> = fs::read_dir(&dir)
            .expect("store dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("favourites.json.corrupt-"))
            .collect();
        assert_eq!(
            preserved.len(),
            1,
            "the corrupt file is kept beside the new store: {preserved:?}"
        );

        // The reset store writes cleanly afterwards.
        set_favourite(&view, &dir.join("kept").display().to_string(), true)
            .expect("write through the reset store");
        let rewritten: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("favourites.json")).expect("reread"))
                .expect("valid JSON after the recovery");
        assert_eq!(rewritten["version"], 1);

        // An unknown version is the same corruption class.
        let dir2 = store_dir();
        fs::write(
            dir2.join("favourites.json"),
            br#"{ "version": 99, "paths": ["/nowhere"] }"#,
        )
        .expect("future version");
        let app2 = build();
        set_store(&app2, &dir2);
        let view2 = webview(&app2);
        let report2 =
            call(&view2, "favourites", serde_json::json!({})).expect("the report still answers");
        assert_eq!(report2["favourites"].as_array().expect("entries").len(), 0);
        assert!(
            report2["notice"]
                .as_str()
                .unwrap_or_default()
                .contains("could not be read"),
            "an unknown version is corruption, not someone else's data: {report2}"
        );
    }

    #[test]
    fn unstarring_removes_the_entry() {
        let holder = tempfile::tempdir().expect("tempdir");
        let target = holder.path().join("starred");
        fs::create_dir(&target).expect("mkdir");

        let app = build();
        set_store(&app, &store_dir());
        let webview = webview(&app);

        let list = set_favourite(&webview, &target.display().to_string(), true).expect("star");
        let stored = list[0]
            .as_str()
            .expect("the store's own string")
            .to_string();

        let after = set_favourite(&webview, &stored, false).expect("unstar with the stored string");
        assert_eq!(after.as_array().expect("list").len(), 0, "the list shrank");

        let report = call(&webview, "favourites", serde_json::json!({})).expect("favourites");
        assert_eq!(
            report["favourites"].as_array().expect("entries").len(),
            0,
            "and the report agrees: {report}"
        );
    }
}
