//! The privileged-image guard: source-level, in the style of C28's markup guard.
//!
//! On Windows the process that runs elevated is the app binary itself (multi-platform
//! release design §3.7.3, §8 risk 4). The only thing standing between the elevated
//! instance and the full UI is `main`'s argv branch running BEFORE any Tauri code.
//! That ordering is one careless refactor away from being lost, and no type system
//! check notices — so this test reads `main.rs`'s own source and pins the order. It is
//! crude on purpose: the claim it guards ("no window, webview or command surface can
//! come into existence on the serve path") is about what text sits in the file, and a
//! textual guard fails the moment someone moves the dispatch below the app launch,
//! names a Tauri type in `main`, or drops the exit-2 refusal for malformed argv.

/// Byte offset of the serve **dispatch statement**, ignoring comments.
///
/// The first version of this guard used `source.find("admin_serve_mode")`, which returns
/// the first occurrence anywhere in the file, comments included. A comment above
/// `run()` that merely named the function therefore satisfied the ordering assertion
/// while the real dispatch sat below the app launch — the guard would stay green while
/// an elevated copy opened the full Tauri application. Requiring the offset to land on a
/// line that both names the function and calls it (`(`) closes that: prose can no longer
/// stand in for code.
///
/// Stated limits: this strips `//` comments by splitting the line, so a `//` inside a
/// string literal would truncate that line; `main.rs` has no such literal, and a guard
/// that parsed Rust properly would be a different (and still refactorable) thing. It
/// pins position for one dispatch call and says nothing about a second one added later.
fn dispatch_index(source: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in source.lines() {
        let at = offset;
        offset += line.len() + 1; // + the newline `lines()` removed
        let code = line.split("//").next().unwrap_or("");
        if code.contains("admin_serve_mode") && code.contains('(') {
            return Some(at);
        }
    }
    None
}

#[test]
fn the_serve_dispatch_precedes_every_tauri_construction_in_main() {
    let source = include_str!("../src/main.rs");

    let dispatch = dispatch_index(source)
        .expect("main must classify argv for the elevated serve branch before anything else");
    let launch = source
        .find("diskguardian_app::run()")
        .expect("main must start the app");
    assert!(
        dispatch < launch,
        "the serve dispatch must precede the app launch: the elevated path must not be \
         one refactor away from initialising Tauri"
    );

    // The branch is the Windows-gated one the design specifies, and a malformed or
    // unexpected argv exits rather than falling through to the app.
    assert!(
        source.contains("#[cfg(target_os = \"windows\")]"),
        "the dispatch must be cfg-gated to Windows"
    );
    assert!(
        source.contains("BadUsage") && source.contains("exit(2)"),
        "a malformed invocation of an elevated copy must exit 2, never open the app"
    );

    // And main must not even be able to name a Tauri construction: if it can, the
    // serve branch is one edit away from building one.
    for banned in ["tauri::", "Builder", "WebviewWindow", "generate_context"] {
        assert!(
            !source.contains(banned),
            "main.rs must not reference {banned:?}: the serve branch must not be able \
             to construct a Tauri surface"
        );
    }
}

#[test]
fn the_guard_rejects_a_dispatch_hidden_behind_a_comment() {
    // The exact shape the old `find()` guard could not tell from the real thing: prose
    // naming the function above the launch, the dispatch itself below it.
    let doctored = r#"
fn main() {
    // Maintenance note: admin_serve_mode dispatch lives at the bottom for now.
    diskguardian_app::run();
    #[cfg(target_os = "windows")]
    match diskguardian_app::admin_serve_mode(std::env::args_os()) {
        diskguardian_app::ServeMode::Normal => {}
        _ => std::process::exit(2),
    }
}
"#;

    let launch = doctored
        .find("diskguardian_app::run()")
        .expect("launch present");

    // The defect, recorded executably: the first-occurrence search this guard used is
    // fooled by the comment, so the ordering assertion would have passed.
    let naive = doctored.find("admin_serve_mode").expect("name present");
    assert!(
        naive < launch,
        "the naive first-occurrence search must be fooled here, or this test is not \
         exercising the defect it exists to pin"
    );

    // The guard as written now sees the real dispatch, below the launch, and fails.
    let dispatch = dispatch_index(doctored).expect("the dispatch call is present in code");
    assert!(
        dispatch > launch,
        "a dispatch hidden below the app launch must be detected as coming after it"
    );
}
