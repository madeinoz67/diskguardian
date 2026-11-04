//! The sidecar binary: argv in, one library call out.
//!
//! The whole measurement protocol — job validation, the serve loop, the result and
//! status writers, and the capability list that goes with them — lives in this
//! crate's library, shared unchanged with the Windows app binary whose elevated argv
//! branch serves the same files (see `src/lib.rs`). This file only parses argv,
//! which is the binary's own concern and has no protocol in it.

use std::path::PathBuf;

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(run())
}

#[derive(Debug)]
enum Mode {
    Usage,
    OneShot(PathBuf),
    Serve {
        dir: PathBuf,
        idle_secs: u64,
        stale_secs: u64,
    },
}

fn parse_args(args: &[std::ffi::OsString]) -> Mode {
    let flag = args.first().map(std::ffi::OsStr::new);
    match flag {
        Some(f) if f == "--job" && args.len() == 2 => Mode::OneShot(PathBuf::from(&args[1])),
        Some(f) if f == "--serve" && args.len() >= 2 => {
            let dir = PathBuf::from(&args[1]);
            let mut idle_secs = diskguardian_admin::DEFAULT_IDLE_SECS;
            let mut stale_secs = diskguardian_admin::DEFAULT_STALE_SECS;
            let mut rest = &args[2..];
            while let Some((head, tail)) = rest.split_first() {
                let value = tail
                    .first()
                    .and_then(|v| v.to_str())
                    .and_then(|v| v.parse().ok());
                match (head.to_str(), value) {
                    (Some("--idle-secs"), Some(v)) => idle_secs = v,
                    (Some("--stale-secs"), Some(v)) => stale_secs = v,
                    _ => return Mode::Usage,
                }
                rest = &tail[1..];
            }
            Mode::Serve {
                dir,
                idle_secs,
                stale_secs,
            }
        }
        _ => Mode::Usage,
    }
}

fn run() -> u8 {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match parse_args(&args) {
        Mode::Usage => {
            eprintln!("usage: diskguardian-admin --job <job.json> | --serve <dir> [--idle-secs N] [--stale-secs N]");
            diskguardian_admin::code::USAGE
        }
        Mode::OneShot(job_path) => diskguardian_admin::one_shot(&job_path),
        Mode::Serve {
            dir,
            idle_secs,
            stale_secs,
        } => diskguardian_admin::serve(&dir, idle_secs, stale_secs),
    }
}
