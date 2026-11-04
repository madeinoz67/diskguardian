pub mod admin;
mod commands;
pub mod favourites;
mod platform;
pub mod serve_mode;
pub mod snapshots;
pub mod state;
pub mod volume;

pub use serve_mode::{admin_serve_mode, ServeMode};

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
            commands::data_folders,
            commands::volumes,
            commands::scan,
            commands::cancel_scan,
            commands::peek_root,
            commands::view,
            commands::collect,
            commands::revalidate,
            commands::trash_stats,
            commands::cleanup,
            commands::duplicates,
            admin::admin_authorised,
            commands::empty_trash,
            commands::collected,
            commands::delete_collected,
            commands::reveal,
            commands::quicklook,
            commands::hidden_space,
            admin::scan_restricted,
            snapshots::local_snapshots,
            snapshots::purge_local_snapshots,
            volume::open_fda_pane,
            favourites::favourites,
            favourites::favourite_marks,
            favourites::favourite_node,
            favourites::set_favourite,
        ])
}

pub fn run() {
    configure(tauri::Builder::default())
        .run(tauri::generate_context!())
        .expect("DiskGuardian failed to start");
}
