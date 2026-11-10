#![windows_subsystem = "windows"]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_haze::init())
        .run(tauri::generate_context!())
        .expect("run");
}
