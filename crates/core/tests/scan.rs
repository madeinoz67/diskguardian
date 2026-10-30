//! Integration tests over real filesystems.
//!
//! Every fixture is built on disk in a temp directory, because the thing under test is
//! the filesystem's own accounting: `len()` versus `st_blocks`, inodes behind hard links,
//! permission bits. A mocked tree would prove nothing.

use diskguardian_core::{build_view, scan, Collector, DeleteMode, NodeKind, ScanOptions, ViewOptions};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn options() -> ScanOptions {
    ScanOptions::default()
}

/// Assert the tree's own arithmetic: no node sums smaller than its children.
fn assert_tree_arithmetic(tree: &diskguardian_core::Tree) {
    for (i, node) in tree.nodes.iter().enumerate() {
        let child_sum: u64 = node.children.iter().map(|&c| tree.nodes[c].total_bytes).sum();
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

#[test]
fn reports_physical_allocation_not_logical_length() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sparse = dir.path().join("sparse.bin");
    {
        // 64 MiB of length, but only the first block is ever written: the rest is holes.
        let file = fs::File::create(&sparse).expect("create");
        file.set_len(64 * 1024 * 1024).expect("set_len");
    }

    let logical = fs::metadata(&sparse).expect("meta").len();
    let tree = scan(dir.path(), &options()).expect("scan");
    let node = tree
        .nodes
        .iter()
        .find(|n| n.name == "sparse.bin")
        .expect("sparse file in tree");

    assert_eq!(logical, 64 * 1024 * 1024, "fixture should be sparse");
    assert!(
        node.total_bytes < logical,
        "reported {} bytes for a {logical}-byte sparse file",
        node.total_bytes
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let expected = fs::metadata(&sparse).expect("meta").blocks() * 512;
        assert_eq!(
            node.total_bytes, expected,
            "reported size must be st_blocks*512"
        );
    }
}

#[test]
fn hard_links_are_counted_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let original = dir.path().join("original.bin");
    fs::write(&original, vec![7u8; 1 << 20]).expect("write");

    for n in 0..3 {
        fs::hard_link(&original, dir.path().join(format!("link-{n}.bin"))).expect("hard link");
    }

    let tree = scan(dir.path(), &options()).expect("scan");

    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(unix)]
    let inode_allocation = fs::metadata(&original).expect("meta").blocks() * 512;
    #[cfg(not(unix))]
    let inode_allocation = 1 << 20;

    // Four names, one inode: exactly one of them may carry the bytes.
    let counted: u64 = tree
        .nodes
        .iter()
        .filter(|n| n.name.ends_with(".bin"))
        .map(|n| n.total_bytes)
        .sum();
    let skipped = tree
        .nodes
        .iter()
        .filter(|n| n.name.ends_with(".bin") && n.link_skipped)
        .count();

    assert_eq!(skipped, 3, "the three duplicates must be marked");
    assert_eq!(
        counted, inode_allocation,
        "the inode must contribute its allocation exactly once"
    );
    assert_eq!(
        tree.total_bytes(),
        counted + dir_own_bytes(&tree),
        "no other bytes may be attributed to the link set"
    );
}

/// The scanned directory's own metadata blocks are legitimately part of the total.
fn dir_own_bytes(tree: &diskguardian_core::Tree) -> u64 {
    tree.nodes[tree.root].own_bytes
}

#[test]
fn unreadable_directory_is_reported_not_dropped() {
    if current_uid() == 0 {
        eprintln!("skipping: running as root, permissions cannot be tested");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).expect("mkdir");
    fs::write(locked.join("secret.txt"), b"hidden").expect("write");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let tree = scan(dir.path(), &options()).expect("scan");

    // Restore so the tempdir can be cleaned up.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");

    let node = tree
        .nodes
        .iter()
        .find(|n| n.name == "locked")
        .expect("the unreadable directory must still be in the tree");

    assert_eq!(node.kind, NodeKind::Restricted);
    assert!(node.note.is_some(), "the reason must be recorded");
    assert!(
        tree.nodes[tree.root].partial,
        "the parent must be flagged partial"
    );
    assert!(tree.errors >= 1, "the error count must be non-zero");
}

fn current_uid() -> u32 {
    // Avoiding a libc dependency for one integer: the real uid comes from `id -u`.
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

#[test]
fn totals_are_consistent_throughout_a_nested_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..3 {
        let sub = dir.path().join(format!("sub-{i}"));
        fs::create_dir(&sub).expect("mkdir");
        fs::write(sub.join("a.txt"), vec![1u8; 4096 * (i + 1)]).expect("write");
        let deeper = sub.join("deeper");
        fs::create_dir(&deeper).expect("mkdir");
        fs::write(deeper.join("b.txt"), vec![2u8; 8192]).expect("write");
    }

    let tree = scan(dir.path(), &options()).expect("scan");
    assert_tree_arithmetic(&tree);
    assert_eq!(tree.file_count(), 6, "six files across three subtrees");
}

#[test]
fn collector_counts_overlapping_items_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).expect("mkdir");
    fs::write(sub.join("big.bin"), vec![1u8; 2 << 20]).expect("write");

    let tree = scan(dir.path(), &options()).expect("scan");
    let sub_id = tree.nodes.iter().position(|n| n.name == "sub").expect("sub");
    let file_id = tree
        .nodes
        .iter()
        .position(|n| n.name == "big.bin")
        .expect("file");

    let mut collector = Collector::new();
    collector.add(sub_id);
    let subtree_total = collector.total(&tree);
    collector.add(file_id);

    assert_eq!(
        collector.total(&tree),
        subtree_total,
        "collecting a child of a collected folder must not add bytes"
    );
    assert_eq!(collector.deletion_order(&tree), vec![sub_id]);
}

#[test]
fn view_merges_the_long_tail_into_smaller_objects() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("huge.bin"), vec![1u8; 4 << 20]).expect("write");
    for i in 0..120 {
        fs::write(dir.path().join(format!("tiny-{i}.bin")), vec![9u8; 64]).expect("write");
    }

    let tree = scan(dir.path(), &options()).expect("scan");
    let view = build_view(
        &tree,
        tree.root,
        &ViewOptions {
            max_depth: 2,
            max_children: 8,
            coverage: 0.999,
        },
    );

    let root = &view[0];
    assert!(
        root.children.len() <= 9,
        "root should hold at most its cap plus one aggregate, got {}",
        root.children.len()
    );
    let aggregate = view
        .iter()
        .find(|n| n.kind == "aggregate")
        .expect("an aggregate node must exist");
    assert!(
        aggregate.aggregated_count > 100,
        "the long tail must be merged, got {}",
        aggregate.aggregated_count
    );
    let child_bytes: u64 = root.children.iter().map(|&c| view[c as usize].bytes).sum();
    assert_eq!(
        child_bytes, root.bytes,
        "the children of a view node must sum to it"
    );
}

#[test]
fn paths_are_reconstructed_from_the_name_chain() {
    let dir = tempfile::tempdir().expect("tempdir");
    let deep = dir.path().join("sub").join("deeper");
    fs::create_dir_all(&deep).expect("mkdir");
    fs::write(deep.join("payload.bin"), b"x").expect("write");

    let tree = scan(dir.path(), &options()).expect("scan");
    let id = tree
        .nodes
        .iter()
        .position(|n| n.name == "payload.bin")
        .expect("payload in tree");

    assert_eq!(tree.path_of(id), deep.join("payload.bin"));
    assert_eq!(tree.path_of(tree.root), dir.path());
    assert_eq!(tree.depth_of(id), 3, "root → sub → deeper → file");
}

#[test]
fn deletion_guard_refuses_protected_locations() {
    let scan_root = Path::new("/Users");
    for protected in [
        "/System",
        "/System/Library",
        "/usr/bin",
        "/usr/lib",
        "/bin",
        "/sbin",
        "/private/etc",
        "/private/var/db",
        "/Library/Apple",
    ] {
        let err = diskguardian_core::delete::guard(Path::new(protected), scan_root);
        assert!(err.is_err(), "{protected} must be refused");
    }
    assert!(diskguardian_core::delete::guard(Path::new("/"), scan_root).is_err());
    assert!(
        diskguardian_core::delete::guard(scan_root, scan_root).is_err(),
        "the scanned root itself must be refused"
    );
}

#[test]
fn deletion_guard_stays_narrow_enough_to_be_useful() {
    // A cleaner that refuses the temp directory and the Homebrew cache cannot do its job.
    let scan_root = Path::new("/Users");
    for allowed in ["/usr/local/Cellar", "/private/tmp", "/Applications/Slack.app"] {
        assert!(
            diskguardian_core::delete::guard(Path::new(allowed), scan_root).is_ok(),
            "{allowed} must remain deletable"
        );
    }
}

#[test]
fn deletion_moves_to_trash_by_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let victim = dir.path().join("collect-me.txt");
    fs::write(&victim, b"delete me").expect("write");

    let report = diskguardian_core::delete::delete(
        &[victim.as_path()],
        dir.path(),
        DeleteMode::Trash,
    );

    assert_eq!(report.removed, 1, "report: {report:?}");
    assert_eq!(report.failed, 0);
    assert!(!victim.exists(), "the item must leave its original location");

    let trashed = dirs_home()
        .map(|home| home.join(".Trash").join("collect-me.txt"))
        .filter(|p| p.exists());
    assert!(
        trashed.is_some(),
        "the item must be recoverable from the Trash"
    );
    if let Some(path) = trashed {
        let _ = fs::remove_file(path);
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

#[test]
fn forgetting_a_deleted_node_updates_every_ancestor() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).expect("mkdir");
    fs::write(sub.join("payload.bin"), vec![3u8; 4 << 20]).expect("write");
    fs::write(dir.path().join("keep.bin"), b"tiny").expect("write");

    let mut tree = scan(dir.path(), &options()).expect("scan");
    let before_root = tree.total_bytes();
    let before_files = tree.file_count();

    let payload = tree
        .nodes
        .iter()
        .position(|n| n.name == "payload.bin")
        .expect("payload");
    let payload_bytes = tree.nodes[payload].total_bytes;
    let sub_id = tree.nodes.iter().position(|n| n.name == "sub").expect("sub");

    tree.forget(&[payload]);

    assert!(
        !tree.nodes[sub_id].children.contains(&payload),
        "the deleted node must leave its parent"
    );
    assert_eq!(
        tree.nodes[sub_id].total_bytes,
        tree.nodes[sub_id].own_bytes,
        "the directory keeps only its own blocks"
    );
    assert_eq!(
        tree.total_bytes(),
        before_root - payload_bytes,
        "every ancestor must lose exactly the deleted bytes"
    );
    assert_eq!(tree.file_count(), before_files - 1);
    assert_tree_arithmetic(&tree);
}

#[test]
fn forgetting_the_scan_root_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("a.bin"), b"x").expect("write");
    let mut tree = scan(dir.path(), &options()).expect("scan");
    let before = tree.total_bytes();
    tree.forget(&[tree.root]);
    assert_eq!(tree.total_bytes(), before, "the root cannot be forgotten");
}

#[test]
fn confirmation_is_required_for_top_level_folders_whatever_their_size() {
    let dir = tempfile::tempdir().expect("tempdir");
    let top = dir.path().join("Desktop");
    fs::create_dir(&top).expect("mkdir");
    fs::write(top.join("small.txt"), b"tiny").expect("write");
    let nested = top.join("cache");
    fs::create_dir(&nested).expect("mkdir");
    fs::write(nested.join("x.bin"), b"x").expect("write");

    let tree = scan(dir.path(), &options()).expect("scan");
    let top_id = tree.nodes.iter().position(|n| n.name == "Desktop").expect("top");
    let nested_id = tree.nodes.iter().position(|n| n.name == "cache").expect("nested");

    // A small top-level folder still asks: it is a whole directory of the user's life.
    assert!(
        diskguardian_core::delete::needs_confirmation(&tree, [top_id], 4096),
        "a top-level folder must confirm even at 4 KB"
    );
    // A small nested folder does not.
    assert!(
        !diskguardian_core::delete::needs_confirmation(&tree, [nested_id], 4096),
        "a small nested folder should not nag"
    );
    // Size still triggers it on its own.
    assert!(diskguardian_core::delete::needs_confirmation(
        &tree,
        [nested_id],
        diskguardian_core::delete::CONFIRM_THRESHOLD_BYTES
    ));
}
