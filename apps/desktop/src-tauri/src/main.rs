#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod desktop;
mod updates;

use aieyes_core::Engine;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

struct Shared(Arc<Mutex<Engine>>, Arc<Mutex<Engine>>);

fn update_events(method: &str, result: &Result<Value, String>) -> Vec<(&'static str, Value)> {
    if method == "hosts.sample" {
        return vec![match result {
            Ok(rows) => ("desktop:hosts", rows.clone()),
            Err(error) => ("desktop:hosts-error", Value::String(error.clone())),
        }];
    }
    if result.is_err() {
        // Sampling may have persisted a pending checkpoint before a later sync fails.
        if matches!(
            method,
            "quotaEstimates.start" | "quotaEstimates.stop" | "quotaEstimates.restart"
        ) {
            return vec![("desktop:data-changed", Value::String(method.into()))];
        }
        return vec![];
    }
    let mut events = vec![];
    if method == "settings.save" {
        // Broadcast invalidation only, never settings or credentials.
        events.push(("desktop:settings", Value::Null));
    }
    if matches!(
        method,
        "settings.save"
            | "sources.scan"
            | "quotas.refresh"
            | "prices.save"
            | "prices.sync"
            | "quotas.order.set"
            | "quotaEstimates.start"
            | "quotaEstimates.stop"
            | "quotaEstimates.restart"
    ) {
        events.push(("desktop:data-changed", Value::String(method.into())));
    }
    events
}

#[tauri::command]
async fn engine_call(
    method: String,
    params: Value,
    state: tauri::State<'_, Shared>,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    if app
        .state::<updates::Updates>()
        .installing
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("正在安装更新，请稍候".into());
    }
    let changed = method.clone();
    let engine = if method.starts_with("hosts.") {
        state.1.clone()
    } else {
        state.0.clone()
    };
    let guarded_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut engine = engine.lock().map_err(|_| "核心连接已断开".to_string())?;
        if guarded_app
            .state::<updates::Updates>()
            .installing
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err("正在安装更新，请稍候".into());
        }
        engine.call(&method, params).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    // Keep both webviews current without starting a second poller or broadcasting
    // settings that may contain credentials, proxy URLs, or shell commands.
    for (event, payload) in update_events(&changed, &result) {
        let _ = app.emit(event, payload);
    }
    result
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let root = std::env::var_os("AIEYES_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            app.manage(Shared(
                Arc::new(Mutex::new(Engine::open(&root)?)),
                Arc::new(Mutex::new(Engine::open(&root)?)),
            ));
            updates::setup(app.handle(), &root);
            desktop::setup(app, &root)?;
            Ok(())
        })
        .on_window_event(desktop::on_window_event)
        .invoke_handler(tauri::generate_handler![
            engine_call,
            updates::updates_info,
            updates::updates_check,
            updates::updates_install,
            updates::updates_preferences,
            updates::updates_later,
            desktop::desktop_info,
            desktop::desktop_mode,
            desktop::desktop_panel,
            desktop::desktop_panel_pin,
            desktop::desktop_panel_page,
            desktop::desktop_panel_cursor_inside,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sampling_failure_invalidates_pending_state_without_claiming_success() {
        let events = update_events("quotaEstimates.stop", &Err("sync failed".into()));
        assert_eq!(
            events,
            vec![(
                "desktop:data-changed",
                Value::String("quotaEstimates.stop".into())
            )]
        );
        assert_eq!(update_events("quotas.order.set", &Ok(Value::Null)).len(), 1);
    }
    #[test]
    fn failed_host_sampling_notifies_the_panel_without_replacing_its_snapshot() {
        assert_eq!(
            update_events("hosts.sample", &Err("核心连接已断开".into())),
            vec![("desktop:hosts-error", json!("核心连接已断开"))]
        );
        let rows = json!([{ "id": "host1", "sample": { "timestamp": 100 } }]);
        assert_eq!(
            update_events("hosts.sample", &Ok(rows.clone())),
            vec![("desktop:hosts", rows)]
        );
    }

    #[test]
    fn failed_mutations_do_not_publish_success_or_configuration_updates() {
        for method in [
            "settings.save",
            "sources.scan",
            "quotas.refresh",
            "prices.save",
        ] {
            assert!(update_events(method, &Err("失败".into())).is_empty());
        }
        assert!(update_events("settings.get", &Ok(json!({ "secret": "private" }))).is_empty());
    }

    #[test]
    fn successful_settings_updates_only_invalidate_configuration() {
        assert_eq!(
            update_events("settings.save", &Ok(json!({ "secret": "private" }))),
            vec![
                ("desktop:settings", Value::Null),
                ("desktop:data-changed", json!("settings.save"))
            ]
        );
    }
}
