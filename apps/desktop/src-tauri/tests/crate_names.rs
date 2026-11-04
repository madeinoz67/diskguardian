//! Every `foo::` path in the app crate's source must be a name the manifest can
//! actually provide.
//!
//! Why this exists: the first Stage-A run of the multi-platform release failed the
//! Windows leg with E0433 — `win_elevate::elevate_serve` in `admin.rs`, while the
//! package is `diskguardian-win-elevate` (crate `diskguardian_win_elevate`). The name
//! lived only inside a `#[cfg(windows)]` function, so no macOS or Linux compile could
//! ever see it; a cross-target `cargo check` cannot run off Windows either (windows-link
//! needs MSVC import libraries). This test is the cheap guard that runs everywhere
//! `cargo test` runs: it parses the app's manifest, derives the set of crate names
//! visible to the crate across ALL dependency sections, and requires every
//! lowercase-initial `name::` path in `src/` to be in that set (or a module, or a
//! single-segment import tail like `io` from `use std::io;`).
//!
//! Stated limits, on purpose: it resolves names against the UNION of targets, so a name
//! that is a macOS-only dependency used inside Windows-gated code would still pass here
//! (the Windows compile remains CI's proof); it does not parse strings, so a crate-like
//! path inside a string literal is invisible to it; and it is textual, so it can
//! false-positive on prose in an inline trailing comment — when it does, fix the comment
//! or the code, never the test's allow-set by hand-editing expectations.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Lowercase-initial identifier immediately followed by `::`, i.e. the shape of a crate
/// or module path prefix. Mid-path segments (after another `::` or a `.`) are not crate
/// names and are skipped by the caller.
fn crate_path_prefixes(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    for (idx, b) in bytes.iter().enumerate() {
        if *b == b':' && idx + 1 < bytes.len() && bytes[idx + 1] == b':' && idx >= 1 {
            // Walk back over the identifier.
            let mut start = idx;
            while start > 0
                && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_')
            {
                start -= 1;
            }
            // A crate path prefix starts at the segment boundary: beginning of line,
            // whitespace, or an opening bracket — NOT another `::` or a `.` (those make
            // it a mid-path segment, e.g. `Win32::UI::Shell` or a method chain).
            let at_boundary = start == 0
                || bytes[start - 1] == b' '
                || bytes[start - 1] == b'\t'
                || bytes[start - 1] == b'('
                || bytes[start - 1] == b'['
                || bytes[start - 1] == b'<'
                || bytes[start - 1] == b'&'
                || bytes[start - 1] == b'!';
            let ident = &line[start..idx];
            let looks_like_crate = ident
                .chars()
                .next()
                .is_some_and(|c| c.is_lowercase() || c == '_')
                && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !ident.is_empty();
            if at_boundary && looks_like_crate {
                found.push(ident.to_string());
            }
        }
    }
    found
}

/// The last segment of a `use` path, plus everything a brace group brings in, so that
/// `io::Error` after `use std::io;` is not mistaken for a crate named `io`.
fn import_tails(use_line: &str) -> Vec<String> {
    let mut tails = Vec::new();
    let body = use_line
        .trim()
        .strip_prefix("use ")
        .and_then(|rest| rest.strip_suffix(';'))
        .unwrap_or("");
    if let Some(brace) = body.find("::{") {
        let parent = &body[..brace];
        let inner = body[brace + 3..].trim_end_matches('}');
        let parent_tail = parent.rsplit("::").next().unwrap_or(parent).to_string();
        for item in inner.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            if item == "self" {
                tails.push(parent_tail.clone());
            } else if let Some((_, alias)) = item.split_once(" as ") {
                tails.push(alias.trim().to_string());
            } else {
                tails.push(item.rsplit("::").next().unwrap_or(item).to_string());
            }
        }
    } else if let Some((_, alias)) = body.split_once(" as ") {
        tails.push(alias.trim().to_string());
    } else {
        tails.push(body.rsplit("::").next().unwrap_or(body).to_string());
    }
    tails
}

fn source_files(src: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(source_files(&path));
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn every_crate_path_prefix_in_src_is_a_name_the_manifest_provides() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml")).expect("read Cargo.toml");

    // Crate names visible to this package: the keys of every dependency section (plain,
    // dev, and every target-specific one), hyphens underscored, plus the package's own
    // lib name. Workspace-inherited deps are spelled `serde.workspace = true`, so the
    // dependency's name is the segment BEFORE the first dot, not the whole key.
    let mut allowed: BTreeSet<String> = [
        "std", "core", "alloc", "crate", "self", "super",
        // Primitive type path roots (`str::from_utf8`, `i64::from`) — std, not crates.
        "str", "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32",
        "u64", "usize",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let mut section = String::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_string();
            continue;
        }
        if !section.contains("dependencies") {
            if section == "[lib]" {
                if let Some(name) = trimmed.strip_prefix("name = ") {
                    allowed.insert(name.trim_matches('"').to_string());
                }
            }
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=') {
            let key = key.trim().split('.').next().unwrap_or("").trim();
            if !key.is_empty() && !key.starts_with('#') {
                allowed.insert(key.replace('-', "_"));
            }
        }
    }

    let src = manifest_dir.join("src");
    let files = source_files(&src);
    assert!(!files.is_empty(), "the app crate has sources to scan");

    // Modules and single-segment imports are legal prefixes without being crates.
    let mut locals: BTreeSet<String> = BTreeSet::new();
    let mut texts = Vec::new();
    for path in &files {
        let text = fs::read_to_string(path).expect("read source");
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("use ") {
                locals.extend(import_tails(trimmed));
            } else if let Some(rest) = trimmed
                .strip_prefix("pub mod ")
                .or_else(|| trimmed.strip_prefix("mod "))
            {
                let name = rest.trim().trim_end_matches(';').trim();
                if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    locals.insert(name.to_string());
                }
            }
        }
        texts.push((path.clone(), text));
    }
    allowed.extend(locals);

    let mut offenders = Vec::new();
    for (path, text) in &texts {
        for (no, line) in text.lines().enumerate() {
            // Pure comment lines (docs and `//`) are narration, not code; inline
            // trailing comments are rare and a false positive there is a finding to
            // resolve, not silence.
            if line.trim_start().starts_with("//") {
                continue;
            }
            for prefix in crate_path_prefixes(line) {
                if !allowed.contains(&prefix) {
                    offenders.push(format!(
                        "{}:{}: `{prefix}::` is not a crate, module or import this \
                         package provides — if the crate exists, the dependency key in \
                         Cargo.toml must match what the code calls it",
                        path.display(),
                        no + 1,
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "unresolvable crate-path prefixes in src/ (the Windows leg of Stage A failed on \
         exactly this class, and no non-Windows compile can see it):\n{}",
        offenders.join("\n")
    );
}
