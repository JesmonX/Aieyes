#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod desktop;

use aieyes_core::Engine;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tauri::Manager;

struct Shared(Arc<Mutex<Engine>>, Arc<Mutex<Engine>>);
#[tauri::command]
async fn engine_call(
    method: String,
    params: Value,
    state: tauri::State<'_, Shared>,
) -> Result<Value, String> {
    let engine = if method.starts_with("hosts.") {
        state.1.clone()
    } else {
        state.0.clone()
    };
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .lock()
            .map_err(|_| "核心连接已断开".to_string())?
            .call(&method, params)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let root = std::env::var_os("AIEYES_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            app.manage(Shared(
                Arc::new(Mutex::new(Engine::open(&root)?)),
                Arc::new(Mutex::new(Engine::open(&root)?)),
            ));
            desktop::setup(app, &root)?;
            Ok(())
        })
        .on_window_event(desktop::on_window_event)
        .invoke_handler(tauri::generate_handler![
            engine_call,
            desktop::desktop_info,
            desktop::desktop_mode,
            desktop::desktop_action
        ])
        .build(tauri::generate_context!())
        .expect("Aieyes startup failed");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            desktop::save(app);
        }
    });
}
