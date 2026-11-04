//! The argv classifier behind Windows self-elevation.
//!
//! On Windows the process that runs elevated is the app binary itself, whose other
//! entry point is the full UI with deletion behind it (multi-platform release design
//! §3.7.3). The classifier is the gate: `main` asks it what this invocation is BEFORE
//! any Tauri code exists, and only a bare program name (`Normal`) may fall through to
//! `diskguardian_app::run()`. Everything else is either the exact serve pair or a
//! refusal (`BadUsage` → exit 2): a malformed or unexpected invocation of an elevated
//! copy must never open an elevated window.
//!
//! The rules, in full:
//!
//! - `[program]` — `Normal`. The app takes no command-line arguments today, so this is
//!   the only shape an ordinary launch has.
//! - `[program, "--dg-admin-serve", <absolute existing directory>]` — `Serve`. The
//!   elevated re-exec the launcher starts. The directory must be absolute (an elevated
//!   process starts in the system directory, not the parent's cwd, so a relative
//!   operand would be resolved against the wrong place in the child) and must exist
//!   and be a directory.
//! - Anything else — `BadUsage`: no argv at all (no real launch has one, and an
//!   elevated copy must not guess), the bare flag without its operand, the flag naming
//!   a directory that is gone or is a file, an unknown flag, extra arguments after the
//!   pair, or bare positional junk.
//!
//! The function is deliberately portable and side-effect free apart from the directory
//! existence check, so its whole contract is tested on every platform, not just
//! Windows.

use std::ffi::OsString;
use std::path::PathBuf;

/// The flag the elevated re-exec is started with. Two argv entries, flag then operand.
pub const SERVE_FLAG: &str = "--dg-admin-serve";

/// What an argv means. See the module docs for the exact rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeMode {
    /// A bare program name: run the app.
    Normal,
    /// The elevated serve pair: serve the admin protocol from this directory and exit
    /// with the session. No window, webview or command surface may be constructed.
    Serve(PathBuf),
    /// Anything unexpected: exit 2 rather than open the app.
    BadUsage,
}

/// Classify an argv (including `argv[0]`, exactly as `std::env::args_os()` yields it).
pub fn admin_serve_mode<I>(args: I) -> ServeMode
where
    I: IntoIterator<Item = OsString>,
{
    let argv: Vec<OsString> = args.into_iter().collect();
    match argv.len() {
        // No argv at all is not a launch shape any platform produces; an elevated
        // copy must not guess what it was asked to be.
        0 => ServeMode::BadUsage,
        // A bare program name is the only ordinary launch the app has.
        1 => ServeMode::Normal,
        // The serve pair: flag plus one absolute, existing directory.
        3 if argv[1] == SERVE_FLAG => match PathBuf::from(&argv[2]) {
            dir if dir.is_absolute() && dir.is_dir() => ServeMode::Serve(dir),
            _ => ServeMode::BadUsage,
        },
        // The bare flag, an unknown flag, positional junk, extra arguments — every
        // other shape is unexpected, and unexpected never opens the app.
        _ => ServeMode::BadUsage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_bare_program_name_is_normal() {
        assert_eq!(
            admin_serve_mode(argv(&["/apps/DiskGuardian.exe"])),
            ServeMode::Normal
        );
    }

    #[test]
    fn the_serve_pair_with_an_absolute_existing_directory_serves() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().canonicalize().expect("absolute");
        assert_eq!(
            admin_serve_mode(argv(&[
                "/apps/DiskGuardian.exe",
                SERVE_FLAG,
                path.to_str().expect("utf8"),
            ])),
            ServeMode::Serve(path)
        );
    }

    #[test]
    fn empty_argv_is_bad_usage() {
        // No real launch is argv-less; an elevated copy must not guess what it is.
        assert_eq!(admin_serve_mode(argv(&[])), ServeMode::BadUsage);
    }

    #[test]
    fn the_bare_flag_without_an_operand_is_bad_usage() {
        assert_eq!(
            admin_serve_mode(argv(&["/apps/DiskGuardian.exe", SERVE_FLAG])),
            ServeMode::BadUsage
        );
    }

    #[test]
    fn the_flag_with_a_nonexistent_directory_is_bad_usage() {
        assert_eq!(
            admin_serve_mode(argv(&[
                "/apps/DiskGuardian.exe",
                SERVE_FLAG,
                "/gone/nowhere/staging",
            ])),
            ServeMode::BadUsage
        );
    }

    #[test]
    fn the_flag_with_a_file_rather_than_a_directory_is_bad_usage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("plain-file.bin");
        std::fs::write(&file_path, b"x").expect("write file");
        let path = file_path.canonicalize().expect("absolute");
        assert_eq!(
            admin_serve_mode(argv(&[
                "/apps/DiskGuardian.exe",
                SERVE_FLAG,
                path.to_str().expect("utf8"),
            ])),
            ServeMode::BadUsage
        );
    }

    #[test]
    fn an_unknown_flag_is_bad_usage() {
        assert_eq!(
            admin_serve_mode(argv(&["/apps/DiskGuardian.exe", "--dg-admin-serv"])),
            ServeMode::BadUsage
        );
        assert_eq!(
            admin_serve_mode(argv(&["/apps/DiskGuardian.exe", "--help"])),
            ServeMode::BadUsage
        );
    }

    #[test]
    fn a_positional_argument_is_bad_usage() {
        // Dragging a file onto the app is not a launch shape the app has; an elevated
        // copy seeing one must not open the UI on it.
        assert_eq!(
            admin_serve_mode(argv(&["/apps/DiskGuardian.exe", "C:\\some\\scan\\root"])),
            ServeMode::BadUsage
        );
    }

    #[test]
    fn anything_after_the_serve_pair_is_bad_usage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().canonicalize().expect("absolute");
        assert_eq!(
            admin_serve_mode(argv(&[
                "/apps/DiskGuardian.exe",
                SERVE_FLAG,
                path.to_str().expect("utf8"),
                "extra",
            ])),
            ServeMode::BadUsage
        );
    }

    #[test]
    fn a_relative_directory_is_bad_usage() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Relative to this process's cwd — exists, but an elevated child resolves argv
        // against the system directory, not this cwd.
        let relative = "relative-staging-probe";
        std::fs::create_dir(dir.path().join(relative)).expect("mkdir");
        assert_eq!(
            admin_serve_mode(argv(&["/apps/DiskGuardian.exe", SERVE_FLAG, relative])),
            ServeMode::BadUsage
        );
    }
}
