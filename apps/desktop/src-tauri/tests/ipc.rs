//! The IPC surface, driven the way the webview drives it.
//!
//! The commands that take more than one argument were only ever exercised by a human
//! clicking them, and one of them (`collect`) was reported as doing nothing at all. This
//! builds the same command surface the app exposes on Tauri's mock runtime and calls them
//! with the exact JSON the frontend sends.

use diskguardian_app::configure;
use diskguardian_app::state::AppState;
use std::fs;
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
    assert!(total >= 64 * 1024, "total should include the nested file, got {total}");
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
    assert!(result.unwrap_err().to_string().contains("cannot be collected"));
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

    let before = call(&webview, "view", serde_json::json!({ "scanId": scan_id, "nodeId": root }))
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
    assert_eq!(struck["missing"], true, "and it is struck through: {report}");
    assert_eq!(struck["bytes"], 0, "a missing node is worth nothing");

    let after_bytes = report["bytes"].as_u64().unwrap_or(0);
    assert!(after_bytes < before_bytes, "the total must shrink");
    assert_eq!(
        before_bytes - after_bytes,
        65536,
        "exactly the deleted subtree, not the directory's metadata size"
    );
}
