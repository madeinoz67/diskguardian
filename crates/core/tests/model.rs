//! Tree surgery: `graft` and `restricted_roots`.
//!
//! The administrator measurement pass (R3) produces trees for paths the unprivileged
//! scan could not read; `graft` is how they rejoin the main tree. These tests use real
//! filesystem fixtures, because the claim under test is that tree arithmetic survives
//! contact with real allocation numbers — a mocked shape would prove nothing.

use diskguardian_core::{scan, GraftError, Node, NodeId, NodeKind, ScanOptions, Tree};

// Only the macOS guard test below reaches this, and an ungated import is a `-D warnings`
// failure on the platforms that skip that test.
#[cfg(target_os = "macos")]
use diskguardian_core::delete;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn options() -> ScanOptions {
    ScanOptions::default()
}

/// The same arithmetic contract `tests/scan.rs` asserts over a fresh walk, asserted here
/// over an operated-on tree: no node sums smaller than its children, and every node
/// accounts for its own bytes.
fn assert_tree_arithmetic(tree: &Tree) {
    for (i, node) in tree.nodes.iter().enumerate() {
        let child_sum: u64 = node
            .children
            .iter()
            .map(|&c| tree.nodes[c].total_bytes)
            .sum();
        assert!(
            node.total_bytes >= child_sum,
            "node {i} ({}) total {} < children {child_sum}",
            node.name,
            node.total_bytes
        );
        assert_eq!(
            node.total_bytes,
            node.own_bytes + child_sum,
            "node {i} ({}) does not account for its own bytes",
            node.name
        );
    }
}

fn current_uid() -> u32 {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(501)
    }
    #[cfg(not(target_os = "macos"))]
    {
        501
    }
}

/// A directory the scanning process cannot enter, holding `files` files of 64 KiB.
#[cfg(unix)]
fn make_locked(base: &Path, name: &str, files: usize) -> PathBuf {
    let dir = base.join(name);
    fs::create_dir(&dir).expect("mkdir");
    for i in 0..files {
        fs::write(
            dir.join(format!("payload-{i:03}.bin")),
            vec![7u8; 64 * 1024],
        )
        .expect("write");
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).expect("chmod");
    dir
}

#[cfg(unix)]
fn unlock(path: &Path) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
}

/// A tree of the same construction as `make_locked`, but readable, scanned as its own
/// root: the "twin" of the restricted directory.
fn readable_twin(files: usize) -> Tree {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..files {
        fs::write(
            dir.path().join(format!("payload-{i:03}.bin")),
            vec![7u8; 64 * 1024],
        )
        .expect("write");
    }
    scan(dir.path(), &options()).expect("scan twin")
}

/// Reproduce exactly what the elevated helper returns for `target`: a tree *of that
/// path*, measured by a process that can read it. The test cannot be root, so it scans
/// a readable twin and relabels the root — which is the helper's own output contract,
/// since the helper scans the restricted path itself, as its root.
fn twin_measured_as(mut twin: Tree, target: &Path) -> Tree {
    twin.scanned_root = target.to_path_buf();
    twin
}

/// AC1: graft flips the Restricted node to a measured Directory, grows every ancestor
/// by the twin's total within tolerance, keeps arithmetic exact, and clears `partial`
/// on ancestors when nothing restricted remains.
#[cfg(unix)]
#[test]
fn graft_replaces_a_restricted_node_and_preserves_arithmetic() {
    if current_uid() == 0 {
        eprintln!("skipping: running as root, permissions cannot be tested");
        return;
    }

    let base = tempfile::tempdir().expect("tempdir");
    let locked = make_locked(base.path(), "locked", 128);
    fs::write(base.path().join("plain.bin"), b"small").expect("write");

    let mut tree = scan(base.path(), &options()).expect("scan");
    let locked_id = tree
        .nodes
        .iter()
        .position(|n| n.name == "locked")
        .expect("locked present");
    assert_eq!(tree.nodes[locked_id].kind, NodeKind::Restricted);
    assert!(tree.nodes[tree.root].partial, "partial before the graft");
    let pre_total = tree.total_bytes();
    let pre_files = tree.file_count();

    let twin = twin_measured_as(readable_twin(128), &locked);
    tree.graft(locked_id, &twin).expect("graft must succeed");

    unlock(&locked);

    let node = &tree.nodes[locked_id];
    assert_eq!(node.kind, NodeKind::Directory, "the petal is measured now");
    assert!(node.note.is_none(), "no stale permission note");
    assert_eq!(
        node.children.len(),
        128,
        "the twin's children were spliced in"
    );

    let delta = tree.total_bytes() - pre_total;
    assert!(
        delta <= twin.total_bytes() + 4096 && delta + 4096 >= twin.total_bytes(),
        "root grew by {delta}, twin total is {} (tolerance 4 KiB)",
        twin.total_bytes()
    );
    assert_eq!(
        tree.file_count(),
        pre_files + 128,
        "file count grows by exactly the twin's files"
    );
    assert!(
        !tree.nodes[tree.root].partial,
        "no restricted node remains, so the root cannot stay partial"
    );
    assert_tree_arithmetic(&tree);
}

/// AC1's second half: a still-restricted sibling keeps every ancestor partial, and the
/// remaining restricted set is exactly the un-grafted node.
#[cfg(unix)]
#[test]
fn partial_stays_set_while_a_sibling_is_still_restricted() {
    if current_uid() == 0 {
        eprintln!("skipping: running as root, permissions cannot be tested");
        return;
    }

    let base = tempfile::tempdir().expect("tempdir");
    let locked = make_locked(base.path(), "locked", 4);
    let locked2 = make_locked(base.path(), "locked2", 2);
    fs::write(base.path().join("plain.bin"), b"small").expect("write");

    let mut tree = scan(base.path(), &options()).expect("scan");
    let locked_id = tree
        .nodes
        .iter()
        .position(|n| n.name == "locked")
        .expect("locked");
    let locked2_id = tree
        .nodes
        .iter()
        .position(|n| n.name == "locked2")
        .expect("locked2");

    tree.graft(locked_id, &twin_measured_as(readable_twin(4), &locked))
        .expect("graft");

    unlock(&locked);
    unlock(&locked2);

    assert_eq!(tree.nodes[locked_id].kind, NodeKind::Directory);
    assert_eq!(tree.nodes[locked2_id].kind, NodeKind::Restricted);
    assert!(
        tree.nodes[tree.root].partial,
        "a still-restricted sibling keeps the root partial"
    );
    assert_eq!(
        tree.restricted_roots(),
        vec![locked2_id],
        "the restricted set after grafting is exactly the un-grafted node"
    );
    assert_tree_arithmetic(&tree);
}

/// Only the *topmost* restricted nodes may be requested: descendants of a Restricted
/// node are invisible, and asking for them twice would double-prompt nothing but
/// double-count results. A fresh walk cannot nest Restricted nodes (they have no
/// children), so the nesting is pinned synthetically.
#[test]
fn restricted_roots_lists_only_topmost_restricted_nodes() {
    fn node(name: &str, kind: NodeKind, parent: Option<NodeId>, children: Vec<NodeId>) -> Node {
        Node {
            name: name.to_string(),
            kind,
            own_bytes: 0,
            total_bytes: 0,
            file_count: 0,
            children,
            parent,
            partial: kind == NodeKind::Restricted,
            link_skipped: false,
            missing: false,
            note: None,
        }
    }

    let tree = Tree {
        nodes: vec![
            node("root", NodeKind::Directory, None, vec![1]),
            node("outer", NodeKind::Restricted, Some(0), vec![2, 3]),
            node("inner", NodeKind::Restricted, Some(1), Vec::new()),
            node("loose", NodeKind::Restricted, Some(0), Vec::new()),
        ],
        root: 0,
        scanned_root: PathBuf::from("/synthetic"),
        duration_ms: 0,
        errors: 0,
    };

    assert_eq!(tree.restricted_roots(), vec![1, 3]);
}

/// The graft contract: the subtree must have been scanned from exactly the path of the
/// node it replaces, compared as strings (both are our own strings; canonicalising here
/// would re-import the `/var` vs `/private/var` mismatch that bit C17).
#[cfg(unix)]
#[test]
fn graft_refuses_a_subtree_scanned_from_a_different_root() {
    let base = tempfile::tempdir().expect("tempdir");
    let locked = make_locked(base.path(), "locked", 2);

    let mut tree = scan(base.path(), &options()).expect("scan");
    let locked_id = tree
        .nodes
        .iter()
        .position(|n| n.name == "locked")
        .expect("locked");
    let pre_total = tree.total_bytes();
    let pre_partial = tree.nodes[tree.root].partial;

    let mut twin = readable_twin(2);
    twin.scanned_root = PathBuf::from("/somewhere/else");
    let err = tree.graft(locked_id, &twin).expect_err("must refuse");

    assert_eq!(
        err,
        GraftError::RootMismatch {
            subtree_root: PathBuf::from("/somewhere/else"),
            node_path: locked.clone(),
        }
    );
    assert_eq!(
        tree.total_bytes(),
        pre_total,
        "a refused graft must not touch the tree"
    );
    assert_eq!(
        tree.nodes[locked_id].kind,
        NodeKind::Restricted,
        "a refused graft must not flip the node"
    );
    assert_eq!(tree.nodes[tree.root].partial, pre_partial);

    unlock(&locked);
}

/// T2 pin: an elevated pass makes protected trees *visible*; visibility is not
/// deletability. The guard is path-based and untouched by R3 — this test exists so a
/// future refactor cannot quietly make that stop being true.
#[cfg(target_os = "macos")]
#[test]
fn guard_refuses_protected_paths_inside_a_grafted_tree() {
    if current_uid() == 0 {
        eprintln!("skipping: running as root, permissions cannot be tested");
        return;
    }

    let base = tempfile::tempdir().expect("tempdir");
    let locked = make_locked(base.path(), "locked", 2);
    fs::write(base.path().join("plain.bin"), b"x").expect("write");

    let mut tree = scan(base.path(), &options()).expect("scan");
    let locked_id = tree
        .nodes
        .iter()
        .position(|n| n.name == "locked")
        .expect("locked");

    // Relabel to where an elevated scan of the startup disk would have attached this
    // subtree: every path in the tree now lives under a protected root.
    tree.scanned_root = PathBuf::from("/System");
    let twin = twin_measured_as(readable_twin(2), &tree.path_of(locked_id));
    tree.graft(locked_id, &twin).expect("graft");
    assert_tree_arithmetic(&tree);

    for id in 0..tree.nodes.len() {
        let path = tree.path_of(id);
        assert!(
            delete::guard(&path, Path::new("/")).is_err(),
            "{} is visible now, and must still be undeletable",
            path.display()
        );
    }

    unlock(&locked);
}
