mod commands;
mod dto;
mod state;

use state::AppState;
use tauri::Manager;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Resolved once and created here, never by an accessor: the
            // platform directories are fixed for the process and the database
            // lives under machine-local data so a roaming-synced WAL cannot
            // corrupt it (paths.rs, spec §1-8).
            let state = AppState::open()?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::search_projects,
            commands::build_install_plan,
            commands::scan_instance,
            commands::preview_toggle,
            commands::execute_toggle,
            commands::list_instances,
            commands::plan_create_instance,
            commands::execute_create_instance,
            commands::link_instance,
            commands::list_pending_creates,
            commands::finish_create,
            commands::abandon_create,
            commands::open_instance_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
