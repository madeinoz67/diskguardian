//! View pruning over real filesystems.
//!
//! Structure is not space. A folder is how the user navigates, so size pruning must never
//! fold a directory into the synthetic "smaller objects" petal just because it holds no
//! bytes: an empty `Documents` is still a place you can go. Files are where the space went,
//! and they are the only thing the tail merge may swallow.
//!
//! Found on a real Windows profile, where `AppData` was 1411 MB of 1411 MB and the thirteen
//! empty folders beside it were all folded away, leaving a home folder that looked like it
//! contained nothing but AppData.
#![allow(dead_code, unused_imports)]

use diskguardian_core::{build_view, scan, ScanOptions, ViewNode, ViewOptions};
use std::fs;
use std::path::Path;

fn view_of(dir: &Path, opts: ViewOptions) -> Vec<ViewNode> {
    let tree = scan(dir, &ScanOptions::default()).expect("scan");
    build_view(&tree, tree.root, &opts)
}

fn shown(view: &[ViewNode]) -> Vec<String> {
    view.iter().map(|n| n.name.clone()).collect()
}

fn aggregate(view: &[ViewNode]) -> Option<&ViewNode> {
    view.iter().find(|n| n.kind == "aggregate")
}

#[test]
fn empty_directories_are_never_folded_into_smaller_objects() {
    // The Windows profile shape: one dominant folder and a row of empty ones beside it.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("huge.bin"), vec![1u8; 8 << 20]).expect("write");
    let folders = [
        "Documents",
        "Downloads",
        "Desktop",
        "Pictures",
        "Videos",
        "Music",
        "Contacts",
        "Favorites",
    ];
    for name in folders {
        fs::create_dir(dir.path().join(name)).expect("mkdir");
    }

    let view = view_of(
        dir.path(),
        ViewOptions {
            max_depth: 2,
            max_children: 64,
            coverage: 0.995,
        },
    );
    let names = shown(&view);

    for name in folders {
        assert!(
            names.iter().any(|n| n == name),
            "`{name}` was folded away; the view holds {names:?}"
        );
    }
}

#[test]
fn the_aggregate_counts_only_files() {
    // Small files past the coverage rule fold; the folders beside them do not, so the
    // aggregate's count must equal the folded file count and no more.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("huge.bin"), vec![1u8; 8 << 20]).expect("write");
    for i in 0..40 {
        fs::write(dir.path().join(format!("tiny-{i}.bin")), vec![9u8; 64]).expect("write");
    }
    for name in ["Documents", "Downloads", "Desktop"] {
        fs::create_dir(dir.path().join(name)).expect("mkdir");
    }

    let view = view_of(
        dir.path(),
        ViewOptions {
            max_depth: 2,
            max_children: 8,
            coverage: 0.995,
        },
    );

    let agg = aggregate(&view).expect("a folded tail");
    // 44 children, of which 3 are folders that must survive. Whatever the cap keeps, the
    // aggregate may only account for files.
    let folders_shown = view
        .iter()
        .filter(|n| ["Documents", "Downloads", "Desktop"].contains(&n.name.as_str()))
        .count();
    assert_eq!(
        folders_shown,
        3,
        "all three folders must survive the tail merge, view holds {:?}",
        shown(&view)
    );
    assert_eq!(
        agg.aggregated_count as usize,
        44 - (view[0].children.len() - 1),
        "the aggregate must count exactly the children that were not kept"
    );
}

#[test]
fn the_child_cap_still_bounds_a_tree_of_directories() {
    // Folders win over files, but the hard cap still wins over both: a directory holding
    // thousands of subdirectories cannot be drawn in full.
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..40 {
        let sub = dir.path().join(format!("sub-{i:02}"));
        fs::create_dir(&sub).expect("mkdir");
        fs::write(sub.join("payload.bin"), vec![7u8; 4096]).expect("write");
    }

    let view = view_of(
        dir.path(),
        ViewOptions {
            max_depth: 2,
            max_children: 8,
            coverage: 0.995,
        },
    );

    let root_children = view[0].children.len();
    assert!(
        root_children <= 9,
        "the cap must bound the root to its limit plus one aggregate, got {root_children}"
    );
    let agg = aggregate(&view).expect("the overflow folds");
    assert!(
        agg.aggregated_count >= 31,
        "the folders past the cap must be accounted for, aggregate counts {}",
        agg.aggregated_count
    );
}

#[test]
fn a_folder_with_no_bytes_is_still_drawn() {
    // A zero-byte directory is invisible in a size-proportional draw; it must still be a
    // node in the view so the list can offer it.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("big.bin"), vec![1u8; 1 << 20]).expect("write");
    fs::create_dir(dir.path().join("Empty")).expect("mkdir");

    let view = view_of(dir.path(), ViewOptions::default());
    let names = shown(&view);
    assert!(
        names.iter().any(|n| n == "Empty"),
        "the empty folder must survive, view holds {names:?}"
    );
}
