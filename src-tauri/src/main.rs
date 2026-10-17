#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn info() -> String {
    format!("core {} on {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![info])
        .run(tauri::generate_context!())
        .expect("failed to run");
}
