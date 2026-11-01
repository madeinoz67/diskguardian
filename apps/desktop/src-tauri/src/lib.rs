mod commands;
mod platform;
mod state;

use state::AppState;

pub fn run() {
    tauri::Builder::default()
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
            commands::empty_trash,
            commands::collected,
            commands::delete_collected,
            commands::reveal,
            commands::quicklook,
        ])
        .run(tauri::generate_context!())
        .expect("DiskGuardian failed to start");
}
