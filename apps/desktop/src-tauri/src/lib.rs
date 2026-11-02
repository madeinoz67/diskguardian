pub mod admin;
mod commands;
mod platform;
pub mod state;

use state::AppState;

/// Everything the app exposes: plugins, state and commands.
///
/// Shared rather than inlined into `run` so a test can build a mock app with exactly the
/// same command surface the real one has. Without that, the multi-argument commands were
/// only ever exercised by a human clicking them.
pub fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::home_dir,
            commands::volumes,
            commands::scan,
            commands::peek_root,
            commands::view,
            commands::collect,
            commands::revalidate,
            commands::trash_stats,
            commands::cleanup,
            admin::admin_authorised,
            commands::empty_trash,
            commands::collected,
            commands::delete_collected,
            commands::reveal,
            commands::quicklook,
            admin::scan_restricted,
        ])
}

pub fn run() {
    configure(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("DiskGuardian failed to start");
}
