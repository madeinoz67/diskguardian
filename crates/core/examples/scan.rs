//! Time a scan and report what it found. Used for the numbers in the ISA; not shipped.
//!
//! cargo run --release -p diskguardian-core --example scan -- ~/Library

use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
    let path = PathBuf::from(&target);

    let started = Instant::now();
    match diskguardian_core::scan(&path, &Default::default()) {
        Ok(tree) => {
            let elapsed = started.elapsed();
            let gb = tree.total_bytes() as f64 / 1024.0 / 1024.0 / 1024.0;
            println!("{target}");
            println!("  wall       {elapsed:?}");
            println!("  engine     {} ms", tree.duration_ms);
            println!("  size       {gb:.2} GB");
            println!("  files      {}", tree.file_count());
            println!("  nodes      {}", tree.nodes.len());
            println!("  errors     {}", tree.errors);
            println!("  partial    {}", tree.nodes[tree.root].partial);
            // The number that keeps the parallel walk honest: a restricted node is space
            // the scan could not measure, so a count that moves with thread count is a
            // defect in the walk, not a wall in the filesystem.
            let restricted = tree
                .nodes
                .iter()
                .filter(|node| node.kind == diskguardian_core::NodeKind::Restricted)
                .count();
            println!("  restricted {restricted}");
        }
        Err(error) => {
            eprintln!("scan failed: {error}");
            std::process::exit(1);
        }
    }
}
