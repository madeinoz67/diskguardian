//! Acceptance tests for the duplicate finder.
//!
//! Every fixture is real files on disk in its own temp directory, because the thing
//! under test is what the pass actually reads: identical bytes, hard-linked inodes,
//! files that differ only in the band between the hashed head and tail. A mocked
//! tree would prove nothing about which files were opened.
//!
//! The progress callback is the seam for that: it fires immediately before every
//! file read, and for no others, so a file absent from the callback log is a file
//! the pass never opened.

use diskguardian_core::{
    find_duplicates, find_duplicates_with_progress, scan, DuplicateOptions, DuplicateProgress,
    DuplicateProgressFn, DuplicateStage, ScanOptions, CONTENT_IDENTICAL_CAVEAT,
};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

fn scan_fixture(root: &std::path::Path) -> diskguardian_core::Tree {
    scan(root, &ScanOptions::default()).expect("scan")
}

/// No floor, no cap, no deadline: the pass must consider everything.
fn every_file() -> DuplicateOptions {
    DuplicateOptions::new(0)
}

/// A progress callback that records every event, so a test can assert which files
/// were opened and at which stage.
fn recorder() -> (
    Arc<Mutex<Vec<DuplicateProgress>>>,
    Option<DuplicateProgressFn>,
) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    (
        log,
        Some(Arc::new(move |event: DuplicateProgress| {
            sink.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        })),
    )
}

fn opened_paths(log: &Arc<Mutex<Vec<DuplicateProgress>>>) -> Vec<std::path::PathBuf> {
    log.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|e| e.path.clone())
        .collect()
}

fn name_of(member: &diskguardian_core::DuplicateMember) -> &str {
    member
        .path
        .file_name()
        .expect("member path has a file name")
        .to_str()
        .expect("member file name is UTF-8")
}

#[test]
fn identical_files_are_grouped_with_wasted_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    // 20 KiB, comfortably above one allocation block and longer than the 8 KiB
    // head/tail reads, so the fixture exercises both cheap stages and the full hash.
    fs::write(dir.path().join("one.bin"), vec![0x11u8; 20 * 1024]).expect("write");
    fs::write(dir.path().join("two.bin"), vec![0x11u8; 20 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    let report = find_duplicates(&tree, &every_file()).expect("report");

    assert_eq!(report.groups.len(), 1, "one duplicate pair expected");
    let group = &report.groups[0];
    assert_eq!(group.members.len(), 2);
    assert_eq!(
        group.bytes_each,
        tree.node(group.members[0].node).total_bytes,
        "bytes_each must be the tree's allocated size for a member"
    );
    assert_eq!(
        group.wasted_bytes,
        group.bytes_each * (group.members.len() as u64 - 1),
        "two members waste exactly one copy"
    );
    let names: Vec<&str> = group.members.iter().map(name_of).collect();
    assert!(names.contains(&"one.bin") && names.contains(&"two.bin"));
    assert!(!report.partial, "nothing stopped the pass early");
}

#[test]
fn same_size_files_differing_between_head_and_tail_are_not_grouped() {
    let dir = tempfile::tempdir().expect("tempdir");
    // 20 KiB files: the head read covers [0, 8 KiB), the tail [12 KiB, 20 KiB), and
    // the middle 4 KiB is covered by neither. A pair that differs ONLY there agrees
    // at both ends — this is the fixture that catches an implementation trusting the
    // edges and skipping the whole-file hash.
    let mut differs_in_middle = vec![0xAAu8; 20 * 1024];
    differs_in_middle[9 * 1024..9 * 1024 + 512].fill(0xBB);
    fs::write(dir.path().join("left.bin"), vec![0xAAu8; 20 * 1024]).expect("write");
    fs::write(dir.path().join("right.bin"), differs_in_middle).expect("write");

    // And a pair that differs at the first byte, which even the cheapest stage
    // should reject.
    let mut differs_at_head = vec![0xCCu8; 20 * 1024];
    differs_at_head[0] = 0xDD;
    fs::write(dir.path().join("front-a.bin"), vec![0xCCu8; 20 * 1024]).expect("write");
    fs::write(dir.path().join("front-b.bin"), differs_at_head).expect("write");

    let tree = scan_fixture(dir.path());
    let report = find_duplicates(&tree, &every_file()).expect("report");

    assert!(
        report.groups.is_empty(),
        "same-size files with different content must never be reported, found {:?}",
        report
            .groups
            .iter()
            .map(|g| g.members.iter().map(name_of).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
}

#[cfg(unix)]
#[test]
fn hard_linked_names_are_excluded_entirely() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Three names, two inodes: `original.bin` and `alias.bin` share one; `copy.bin`
    // is a separate inode with identical bytes. None of the three may be proposed.
    let original = dir.path().join("original.bin");
    fs::write(&original, vec![0x22u8; 10 * 1024]).expect("write");
    fs::hard_link(&original, dir.path().join("alias.bin")).expect("hard link");
    fs::write(dir.path().join("copy.bin"), vec![0x22u8; 10 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    // min_bytes 0 so the zero-byte link_skipped name is not filtered out for free —
    // the exclusion itself is what must keep it out of a group.
    let report = find_duplicates(&tree, &every_file()).expect("report");

    assert!(
        report.groups.is_empty(),
        "a hard-linked inode is not a duplicate, however many names it has"
    );
    assert_eq!(
        report.excluded_links, 1,
        "the byte-carrying name of the shared inode is the one excluded at candidate time"
    );
}

#[test]
fn files_below_min_bytes_are_never_hashed() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("tiny-a.bin"), vec![0x33u8; 1024]).expect("write");
    fs::write(dir.path().join("tiny-b.bin"), vec![0x33u8; 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    let (log, progress) = recorder();
    // 8 KiB is above the pair's whole allocation, so both fall below the floor.
    let opts = DuplicateOptions {
        min_bytes: 8 * 1024,
        ..every_file()
    };
    let report = find_duplicates_with_progress(&tree, &opts, progress).expect("report");

    assert!(report.groups.is_empty());
    assert_eq!(report.files_hashed, 0, "below the floor means never read");
    assert!(
        opened_paths(&log).is_empty(),
        "the progress seam must show no reads at all"
    );
    assert!(!report.partial, "a floor is a complete answer, not a stop");

    // Positive control: the same pair with no floor is found, so it is the floor
    // that excluded them, not a broken fixture.
    let control = find_duplicates(&tree, &every_file()).expect("report");
    assert_eq!(
        control.groups.len(),
        1,
        "fixture pair is findable without the floor"
    );
}

#[test]
fn a_file_without_a_size_twin_is_never_opened() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Allocations chosen to land in different blocks: 40 KiB, 100 KiB, and a 6 KiB
    // pair. Neither singleton has an allocation twin, so neither may even be opened.
    fs::write(dir.path().join("solo-a.bin"), vec![0x44u8; 40 * 1024]).expect("write");
    fs::write(dir.path().join("solo-b.bin"), vec![0x55u8; 100 * 1024]).expect("write");
    fs::write(dir.path().join("twin-x.bin"), vec![0x66u8; 6 * 1024]).expect("write");
    fs::write(dir.path().join("twin-y.bin"), vec![0x66u8; 6 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    let (log, progress) = recorder();
    let report = find_duplicates_with_progress(&tree, &every_file(), progress).expect("report");

    let opened = opened_paths(&log);
    assert!(
        opened
            .iter()
            .any(|p| p.ends_with("twin-x.bin") || p.ends_with("twin-y.bin")),
        "the seam fires for files that are opened, or it proves nothing"
    );
    for solo in ["solo-a.bin", "solo-b.bin"] {
        assert!(
            opened.iter().all(|p| !p.ends_with(solo)),
            "{solo} has no size twin and must never be opened"
        );
    }
    // The seam reports once per stage, and a verified duplicate is read at two
    // stages (edges, then whole file) — so the distinct-file count is the number
    // of head/tail events, not the number of events.
    let edge_reads = log
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter(|e| e.stage == DuplicateStage::HeadTail)
        .count();
    assert_eq!(edge_reads, 2, "exactly the twinned pair is read");
    assert!(
        opened
            .iter()
            .all(|p| p.ends_with("twin-x.bin") || p.ends_with("twin-y.bin")),
        "no file outside the pair may be opened at any stage"
    );
    assert_eq!(report.files_hashed, 2);
    assert_eq!(report.groups.len(), 1);
}

#[test]
fn an_expired_deadline_returns_a_partial_report_without_reading() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("one.bin"), vec![0x77u8; 20 * 1024]).expect("write");
    fs::write(dir.path().join("two.bin"), vec![0x77u8; 20 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    let (log, progress) = recorder();
    let opts = DuplicateOptions {
        deadline: Some(Instant::now() - Duration::from_secs(1)),
        ..every_file()
    };
    let report = find_duplicates_with_progress(&tree, &opts, progress).expect("report");

    assert!(
        report.partial,
        "a spent deadline must be visible as partial, not passed off as complete"
    );
    assert!(report.groups.is_empty());
    assert_eq!(
        report.files_hashed, 0,
        "the pass must stop before reading anything"
    );
    assert!(
        opened_paths(&log).is_empty(),
        "no file may be opened after the deadline is spent"
    );
}

#[test]
fn reaching_the_candidate_cap_marks_the_report_partial() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(dir.path().join(name), vec![0x88u8; 6 * 1024]).expect("write");
    }

    let tree = scan_fixture(dir.path());
    let (log, progress) = recorder();
    let opts = DuplicateOptions {
        max_candidates: Some(2),
        ..every_file()
    };
    let report = find_duplicates_with_progress(&tree, &opts, progress).expect("report");

    assert!(report.partial, "the third candidate went unexamined");
    assert_eq!(
        report.files_hashed, 2,
        "the cap counts candidate files read"
    );
    assert_eq!(
        report.groups.len(),
        1,
        "the two hashed files do form a group"
    );
    let names: Vec<&str> = report.groups[0].members.iter().map(name_of).collect();
    assert_eq!(names.len(), 2);
    // Scan order is alphabetical, so the cap stops at a.bin and b.bin; asserting the
    // members by name also pins that c.bin was never smuggled into the group.
    assert!(names.contains(&"a.bin") && names.contains(&"b.bin"));
    assert!(
        opened_paths(&log).iter().all(|p| !p.ends_with("c.bin")),
        "the capped candidate is never opened"
    );
}

#[test]
fn every_group_carries_the_shared_storage_caveat() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("one.bin"), vec![0x99u8; 20 * 1024]).expect("write");
    fs::write(dir.path().join("two.bin"), vec![0x99u8; 20 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    let report = find_duplicates(&tree, &every_file()).expect("report");

    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(group.caveat, CONTENT_IDENTICAL_CAVEAT);
    let caveat = group.caveat.to_lowercase();
    assert!(
        caveat.contains("content-identical"),
        "the caveat must say what a match means, not just what it might cost"
    );
    assert!(
        caveat.contains("share"),
        "the caveat must warn that members may share physical storage"
    );
}

#[test]
fn the_newest_file_is_the_kept_one_and_only_one_is() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in ["a.bin", "b.bin", "c.bin"] {
        fs::write(dir.path().join(name), vec![0xABu8; 6 * 1024]).expect("write");
    }
    let now = SystemTime::now();
    let age = |days: u64| now - Duration::from_secs(days * 86_400);
    for (name, days) in [("a.bin", 1), ("b.bin", 0), ("c.bin", 2)] {
        fs::File::options()
            .write(true)
            .open(dir.path().join(name))
            .expect("open for mtime")
            .set_modified(age(days))
            .expect("set mtime");
    }

    let tree = scan_fixture(dir.path());
    let report = find_duplicates(&tree, &every_file()).expect("report");

    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(
        group.members.iter().filter(|m| m.keep).count(),
        1,
        "exactly one keeper per group"
    );
    let keeper = group
        .members
        .iter()
        .find(|m| m.keep)
        .expect("a keeper exists");
    assert_eq!(
        name_of(keeper),
        "b.bin",
        "the newest file is the one to keep; the stale copies are the duplicates"
    );
}

#[test]
fn a_gone_scan_root_is_an_error_not_a_silent_empty_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("one.bin"), vec![0xBCu8; 20 * 1024]).expect("write");
    fs::write(dir.path().join("two.bin"), vec![0xBCu8; 20 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    drop(dir); // the fixture, root included, is gone

    let err = find_duplicates(&tree, &every_file())
        .expect_err("a vanished root must not be reported as \"no duplicates\"");
    assert_eq!(err.path, tree.scanned_root);
}

#[test]
fn a_file_that_became_a_symlink_since_the_scan_is_excluded() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("one.bin"), vec![0xCDu8; 6 * 1024]).expect("write");
    fs::write(dir.path().join("two.bin"), vec![0xCDu8; 6 * 1024]).expect("write");

    let tree = scan_fixture(dir.path());
    // Replace one member with a symlink to the other. Following it would read
    // through the link and out of what the tree actually scanned; the pass must
    // drop the file instead.
    fs::remove_file(dir.path().join("one.bin")).expect("remove");
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.path().join("two.bin"), dir.path().join("one.bin"))
        .expect("symlink");

    let report = find_duplicates(&tree, &every_file()).expect("report");

    assert!(
        report.groups.is_empty(),
        "a group whose member is no longer the scanned file must not be reported"
    );
    #[cfg(unix)]
    assert_eq!(
        report.errors, 1,
        "the replaced member is counted, not hidden"
    );
}
