mod commands;
mod dto;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::start_demo_download,
            commands::search_projects,
            commands::build_install_plan,
            commands::scan_instance,
            commands::preview_toggle,
            commands::execute_toggle,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
