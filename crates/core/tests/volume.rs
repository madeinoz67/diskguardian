//! Volume-scan behaviour (C29): the device/prefix boundary and the hidden-space
//! residual node.
//!
//! Boundary fixtures are real filesystem objects in temp directories, because the thing
//! under test is the filesystem's own accounting. The device test additionally uses the
//! OS's own nested mounts under `/System/Volumes` — Apple's fixed system paths, the one
//! portable source of a genuinely foreign `st_dev` — and asserts its own premise (the
//! devices really differ) so it can never pass vacuously on a machine where they don't.
//!
//! Platform-gated tests live in this file, so on another OS some imports are legitimately
//! unused; `dead_code` and `unused_imports` are allowed HERE and only here, for the same
//! three-CI-rounds reason as `tests/scan.rs`. Production code keeps `-D warnings`.
#![allow(dead_code, unused_imports)]

use diskguardian_core::{
    build_view, build_volume_view, scan, ScanOptions, ViewOptions, VolumeAnchor,
};
use std::fs;
use std::path::PathBuf;

/// Assert the tree's own arithmetic, the same probe `tests/scan.rs` uses: excluded and
/// skipped subtrees must not disturb it, and nothing may be quietly zeroed.
fn assert_tree_arithmetic(tree: &diskguardian_core::Tree) {
    for (i, node) in tree.nodes.iter().enumerate() {
        let child_sum: u64 = node
            .children
            .iter()
            .map(|&c| tree.nodes[c].total_bytes)
            .sum();
        assert_eq!(
            node.total_bytes,
            node.own_bytes + child_sum,
            "node {i} ({}) does not account for its own bytes",
            node.name
        );
    }
}

/// A scan of a small real fixture, with the size of the tree it produced.
fn scanned_fixture() -> (tempfile::TempDir, diskguardian_core::Tree) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("a.bin"), vec![1u8; 16 * 1024]).expect("write");
    fs::create_dir_all(dir.path().join("nested")).expect("mkdir");
    fs::write(dir.path().join("nested/b.bin"), vec![2u8; 32 * 1024]).expect("write");
    let tree = scan(dir.path(), &ScanOptions::default()).expect("scan");
    (dir, tree)
}

// ─── the boundary ─────────────────────────────────────────────────────────────────────

#[test]
fn an_excluded_prefix_leaves_the_tree_and_is_listed() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("kept.bin"), vec![1u8; 4096]).expect("write");
    fs::create_dir_all(dir.path().join("skipme")).expect("mkdir");
    fs::write(dir.path().join("skipme/inner.bin"), vec![2u8; 8192]).expect("write");

    let opts = ScanOptions {
        exclude_prefixes: vec![dir.path().join("skipme")],
        ..Default::default()
    };
    let tree = scan(dir.path(), &opts).expect("scan");

    let names: Vec<&str> = tree.nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(
        !names.contains(&"skipme") && !names.contains(&"inner.bin"),
        "an excluded subtree is not part of the tree: {names:?}"
    );
    // A3's shape: excluded is reported, never silently absent from the record.
    assert_eq!(
        tree.skipped_mounts,
        vec![dir.path().join("skipme").display().to_string()],
        "the exclusion is listed"
    );
    assert!(tree.total_bytes() > 0, "the kept content is still measured");
    assert_tree_arithmetic(&tree);
}

/// The prefix match is by path components, not by string prefix: excluding `.../skip`
/// must not swallow `.../skipme` beside it. A `starts_with` on strings would.
#[test]
fn prefix_matching_is_by_components_not_characters() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["skip", "skipme"] {
        fs::create_dir_all(dir.path().join(name)).expect("mkdir");
        fs::write(dir.path().join(name).join("inner.bin"), vec![3u8; 4096]).expect("write");
    }

    let opts = ScanOptions {
        exclude_prefixes: vec![dir.path().join("skip")],
        ..Default::default()
    };
    let tree = scan(dir.path(), &opts).expect("scan");

    let names: Vec<&str> = tree.nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(
        !names.contains(&"skip"),
        "the excluded prefix itself is gone"
    );
    assert!(
        names.contains(&"skipme"),
        "a sibling sharing the first characters is NOT excluded: {names:?}"
    );
    assert_eq!(
        tree.skipped_mounts.len(),
        1,
        "exactly one exclusion: {:?}",
        tree.skipped_mounts
    );
}

/// A foreign `st_dev` is the real boundary for a volume scan. Only the OS can provide a
/// genuinely foreign device, so the fixture is Apple's own nested mounts: `/System/Volumes`
/// sits on the boot group's device, `Preboot` does not, `Data` does (which is exactly why
/// the app supplies it as an exclusion prefix rather than relying on the device bound).
/// macOS-gated with its imports; the premise is asserted, not assumed.
///
/// The walk is bounded by a `ScanOptions` deadline — the options-object budget, not a
/// wall-clock hope — so a walker that ignores the boundary (as this test once caught a
/// skeleton doing) enters the whole Data volume and still terminates.
#[cfg(target_os = "macos")]
#[test]
fn a_foreign_device_subtree_is_not_walked_and_is_listed() {
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    let root = Path::new("/System/Volumes");
    let preboot = root.join("Preboot");
    let data = root.join("Data");
    let root_dev = fs::metadata(root).expect("stat /System/Volumes").dev();
    let preboot_dev = fs::metadata(&preboot)
        .expect("stat Preboot (Apple's fixed path)")
        .dev();
    let data_dev = fs::metadata(&data)
        .expect("stat Data (Apple's fixed path)")
        .dev();
    assert_ne!(
        preboot_dev, root_dev,
        "premise: Preboot really is a foreign device"
    );
    assert_eq!(
        data_dev, root_dev,
        "premise: Data shares the boot group's device, so only a prefix can exclude it"
    );

    let opts = ScanOptions {
        same_device_only: true,
        exclude_prefixes: vec![data.clone()],
        deadline: Some(Instant::now() + Duration::from_secs(180)),
        ..Default::default()
    };
    let tree = scan(root, &opts).expect("scan");

    let names: Vec<&str> = tree.nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(
        !names.contains(&"Preboot"),
        "a foreign device is another volume's content, not this scan's: {names:?}"
    );
    let skipped = tree.skipped_mounts.clone();
    assert!(
        skipped.contains(&preboot.display().to_string()),
        "the foreign mount is listed, never silently dropped: {skipped:?}"
    );
    assert!(
        skipped.contains(&data.display().to_string()),
        "the prefix-excluded Data mount is listed too: {skipped:?}"
    );
    assert_tree_arithmetic(&tree);
}

// ─── the residual node ────────────────────────────────────────────────────────────────

#[test]
fn the_residual_is_placed_exactly() {
    let (_guard, tree) = scanned_fixture();
    let k: u64 = 12_345;
    let anchor = VolumeAnchor {
        used_bytes: tree.total_bytes() + k,
    };
    let view = build_volume_view(&tree, tree.root, &ViewOptions::default(), &anchor);

    let hidden = view
        .nodes
        .iter()
        .find(|n| n.kind == "hidden")
        .expect("a positive residual draws exactly one hidden node");
    assert_eq!(
        hidden.bytes, k,
        "the petal's bytes are the residual, exactly"
    );
    assert_eq!(
        hidden.node_id, None,
        "synthetic: it can never be collected, navigated or revealed"
    );
    assert_eq!(view.nodes[0].bytes, anchor.used_bytes);
    assert_eq!(
        view.residual_bytes, k as i64,
        "the residual is reported to the caller, exactly"
    );
    // The ring plus the root's own blocks sums to the volume.
    //
    // The root directory's own allocation is part of the tree's total but has no petal:
    // children sum to `total - own`. On APFS an empty directory occupies zero blocks, so
    // this assertion passed on macOS while being wrong; on ext4 the same directory costs
    // 4096 and the ring came up a block short. Written against the filesystem's own
    // arithmetic, it holds on all three.
    let child_sum: u64 = view.nodes[0]
        .children
        .iter()
        .map(|&c| view.nodes[c as usize].bytes)
        .sum();
    let root_own = tree.nodes[tree.root].own_bytes;
    assert_eq!(
        child_sum + root_own,
        anchor.used_bytes,
        "children, the hidden petal and the root's own blocks tile the anchor \
         (for a positive residual; with a negative one there is no petal and the ring \
         tiles the tree total instead)"
    );
}

/// The RED control for the mechanism: the ordinary projection never invents a hidden
/// node, so a folder scan cannot show one.
#[test]
fn without_an_anchor_there_is_no_hidden_node() {
    let (_guard, tree) = scanned_fixture();
    let view = build_view(&tree, tree.root, &ViewOptions::default());
    assert!(
        view.iter().all(|n| n.kind != "hidden"),
        "no anchor, no hidden node"
    );
}

#[test]
fn a_negative_residual_is_surfaced_not_drawn() {
    let (_guard, tree) = scanned_fixture();
    let k: u64 = 4096;
    let anchor = VolumeAnchor {
        used_bytes: tree.total_bytes() - k,
    };
    let view = build_volume_view(&tree, tree.root, &ViewOptions::default(), &anchor);

    assert!(
        view.nodes.iter().all(|n| n.kind != "hidden"),
        "a negative residual never draws a zero-width petal"
    );
    assert_eq!(
        view.residual_bytes,
        -(k as i64),
        "the negative residual is reported in the return, never clamped to zero silently"
    );
    assert_eq!(view.nodes[0].bytes, anchor.used_bytes);
}

#[test]
fn a_zero_residual_draws_no_petal() {
    let (_guard, tree) = scanned_fixture();
    let anchor = VolumeAnchor {
        used_bytes: tree.total_bytes(),
    };
    let view = build_volume_view(&tree, tree.root, &ViewOptions::default(), &anchor);
    assert!(
        view.nodes.iter().all(|n| n.kind != "hidden"),
        "nothing hidden, nothing to draw"
    );
    assert_eq!(view.residual_bytes, 0);
}

/// `Tree` crosses the process boundary to and from the administrator helper's job and
/// results files, which were written before this field existed. A missing field must
/// default, not fail the graft.
#[test]
fn a_tree_from_before_this_field_still_deserializes() {
    let (_guard, tree) = scanned_fixture();
    let mut doc = serde_json::to_value(&tree).expect("serialize");
    doc.as_object_mut()
        .expect("a tree serializes as an object")
        .remove("skipped_mounts");
    let back: diskguardian_core::Tree =
        serde_json::from_value(doc).expect("an old-shape tree deserializes");
    assert!(back.skipped_mounts.is_empty());
}

/// The excluded-record survives a serialize round trip, so the app's panel can name what
/// the boundary stopped.
#[test]
fn skipped_mounts_survive_serialization() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("edge")).expect("mkdir");
    fs::write(dir.path().join("edge/x.bin"), vec![4u8; 4096]).expect("write");

    let opts = ScanOptions {
        exclude_prefixes: vec![dir.path().join("edge")],
        ..Default::default()
    };
    let tree = scan(dir.path(), &opts).expect("scan");
    let back: diskguardian_core::Tree =
        serde_json::from_value(serde_json::to_value(&tree).expect("serialize")).expect("back");
    assert_eq!(back.skipped_mounts, tree.skipped_mounts);
}
