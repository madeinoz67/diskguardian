//! The C29 acceptance harness: a bounded volume scan of a mount, cross-checked against
//! the probes the panel uses. Used for the real-machine numbers in the ISA; not
//! shipped.
//!
//! cargo run --release -p diskguardian-app --example hidden_measure -- /
//!
//! The band check prints the design's pre-committed interval
//! ([0.9 × other volumes, 0.25 × used]) alongside the residual, but prints no verdict
//! of its own: the verdict belongs to the acceptance rule, read with eyes, not baked
//! into the harness.

use diskguardian_app::volume;
use std::path::PathBuf;

fn main() {
    let target = std::env::args().nth(1).unwrap_or_else(|| "/".to_string());
    let root = PathBuf::from(&target);

    let started = std::time::Instant::now();
    let opts = diskguardian_core::ScanOptions {
        same_device_only: true,
        exclude_prefixes: volume::volume_scan_exclusions(),
        ..Default::default()
    };
    let tree = match diskguardian_core::scan(&root, &opts) {
        Ok(tree) => tree,
        Err(error) => {
            eprintln!("scan failed: {error}");
            std::process::exit(1);
        }
    };
    let wall = started.elapsed();

    let used = volume::volume_used(&root);
    let container = volume::container_report(&root);
    let purgeable = volume::purgeable_bytes(&root);

    println!("{target}");
    println!("  wall          {wall:?}");
    println!("  engine        {} ms", tree.duration_ms);
    println!("  nodes         {}", tree.nodes.len());
    println!("  tree_bytes    {}", tree.total_bytes());
    println!("  used_bytes    {used:?}");
    println!("  purgeable     {purgeable:?}");
    println!("  skipped       {}", tree.skipped_mounts.len());
    for skipped in &tree.skipped_mounts {
        println!("    excluded    {skipped}");
    }

    let Some(used) = used else {
        println!("  residual      unavailable: no anchor for this mount");
        return;
    };
    let residual = used as i128 - tree.total_bytes() as i128;
    println!("  residual      {residual}");

    let Some(report) = container else {
        println!("  decomposition none (unmappable root or the parse refused)");
        return;
    };
    match volume::decompose_other_volumes(std::slice::from_ref(&report), &root) {
        Some((container_ref, others)) => {
            let sum: u128 = others.iter().map(|v| v.used_bytes as u128).sum();
            for v in &others {
                println!(
                    "    other       {:>15}  {} [{}] ({})",
                    v.used_bytes,
                    v.name,
                    v.roles.join(","),
                    v.device
                );
            }
            println!("  container     {container_ref}");
            println!("  others_sum    {sum}");
            println!("  still_hidden  {}", residual - sum as i128);
            println!(
                "  band          [{} , {}]",
                (sum as f64 * 0.9) as u128,
                used / 4
            );
        }
        None => println!("  decomposition none for this root"),
    }
}
