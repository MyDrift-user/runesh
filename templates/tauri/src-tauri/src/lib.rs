use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use tauri::Manager;

// ── App state ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub server: String,
    pub api_key: String,
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
}

// ── Commands ────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(
    state: tauri::State<'_, AppState>,
    server: String,
    api_key: String,
) -> Result<String, String> {
    let mut config = state.config.lock().unwrap();
    config.server = server;
    config.api_key = api_key;
    runesh_tauri::config::save_config("YOUR_APP", &*config)?;
    Ok("Config saved".into())
}

#[tauri::command]
fn get_status() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "connected": false,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

// ── App setup ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config: AppConfig = runesh_tauri::config::load_or_create("YOUR_APP");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            config: Mutex::new(config),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_status,
        ])
        .setup(|app| {
            // System tray
            runesh_tauri::tray::setup_tray(
                app.handle(),
                include_bytes!("../icons/icon.png"),
                &[
                    ("Show", "show"),
                    ("Quit", "quit"),
                ],
            )?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
