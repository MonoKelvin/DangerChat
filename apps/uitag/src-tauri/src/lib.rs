pub mod cmd;
pub mod coords;
pub mod export;
pub mod import;
pub mod propagate;
pub mod state;
pub mod tags;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            cmd::list_images,
            cmd::load_tags,
            cmd::save_state,
            cmd::load_state,
            cmd::export_zip,
            cmd::propagate_boxes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
