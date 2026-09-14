pub mod cmd;
pub mod coords;
pub mod detect;
pub mod export;
pub mod import;
pub mod propagate;
pub mod state;
pub mod tags;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            cmd::open_workspace,
            cmd::load_tags,
            cmd::save_state,
            cmd::export_zip,
            cmd::propagate_boxes,
            cmd::reveal_path,
            cmd::detect_lines,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
