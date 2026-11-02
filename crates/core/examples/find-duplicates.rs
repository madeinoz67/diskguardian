//! Scan a path and report its content-identical file groups.
//!
//!     cargo run -p diskguardian-core --release --example find-duplicates -- <path> [min-bytes]
//!
//! A small, honest demonstration of the duplicate pass: it prints the tree's size
//! and file count, how long the scan and the duplicate pass each took, and every
//! group with its caveat — the caveat is not optional decoration, which is the
//! point of printing it here.

use diskguardian_core::{find_duplicates, scan, DuplicateOptions, ScanOptions};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: find-duplicates <path> [min-bytes]");
        return ExitCode::from(2);
    };
    let min_bytes = args.next().map_or(1024 * 1024, |s| {
        s.parse().unwrap_or_else(|_| {
            eprintln!("min-bytes must be a number of bytes");
            std::process::exit(2);
        })
    });

    let started = Instant::now();
    let tree = match scan(Path::new(&root), &ScanOptions::default()) {
        Ok(tree) => tree,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let scan_secs = started.elapsed().as_secs_f64();
    println!(
        "scanned {} files, {}, in {scan_secs:.2}s",
        tree.file_count(),
        human(tree.total_bytes())
    );

    let started = Instant::now();
    let report = match find_duplicates(&tree, &DuplicateOptions::new(min_bytes)) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let pass_secs = started.elapsed().as_secs_f64();

    let wasted: u64 = report.groups.iter().map(|g| g.wasted_bytes).sum();
    println!(
        "{} groups, {} possibly wasted ({} files hashed, {} links excluded, {} errors) in {pass_secs:.2}s",
        report.groups.len(),
        human(wasted),
        report.files_hashed,
        report.excluded_links,
        report.errors
    );
    if report.partial {
        println!("PARTIAL: the pass stopped early; more duplicates may exist");
    }
    for group in &report.groups {
        println!(
            "\n{} each, {} if none share storage",
            human(group.bytes_each),
            human(group.wasted_bytes)
        );
        println!("  caveat: {}", group.caveat);
        for member in &group.members {
            println!(
                "  {} [{}]",
                member.path.display(),
                if member.keep { "KEEP" } else { "duplicate" }
            );
        }
    }

    ExitCode::SUCCESS
}
