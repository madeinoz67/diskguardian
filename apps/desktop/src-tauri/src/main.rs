#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before ANY tauri code: an elevated re-exec serves the admin protocol and exits.
    // No window, no webview, no command surface can come into existence —
    // `tests/serve_order.rs` pins this ordering textually, because nothing else would
    // notice it moving (multi-platform release design §3.7.3). The classifier is
    // strict on purpose: anything but a bare program name or the exact serve pair is
    // BadUsage, so a malformed invocation of an elevated copy exits 2 rather than
    // opening the full app with administrator rights.
    #[cfg(target_os = "windows")]
    match diskguardian_app::admin_serve_mode(std::env::args_os()) {
        diskguardian_app::ServeMode::Normal => {}
        diskguardian_app::ServeMode::Serve(dir) => {
            std::process::exit(diskguardian_admin::serve_standalone(&dir) as i32)
        }
        diskguardian_app::ServeMode::BadUsage => std::process::exit(2),
    }
    diskguardian_app::run();
}
