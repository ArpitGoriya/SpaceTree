mod commands;
mod dto;
mod state;
mod volumes;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_volumes,
            commands::pick_folder,
            commands::start_scan,
            commands::cancel_scan,
            commands::fast_scan_status,
            commands::request_elevation,
            commands::list_children,
            commands::node_info,
            commands::search,
            commands::treemap_layout,
            commands::export_markdown_text,
            commands::save_text_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
