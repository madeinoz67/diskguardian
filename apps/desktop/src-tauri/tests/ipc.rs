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
