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

#[test]
fn the_serve_dispatch_precedes_every_tauri_construction_in_main() {
    let source = include_str!("../src/main.rs");

    let dispatch = source
        .find("admin_serve_mode")
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
