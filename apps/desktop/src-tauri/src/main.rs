#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(target_os = "windows")]
mod capsule;
mod desktop;
mod phase_colors;
#[cfg(target_os = "windows")]
mod passive_window;
mod updates;

use aieyes_core::Engine;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

fn local_path(raw: &str) -> std::path::PathBuf {
    if (raw == "~" || raw.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return std::path::PathBuf::from(home).join(raw.strip_prefix("~/").unwrap_or(""));
    }
    raw.into()
}

#[tauri::command]
async fn select_local_path(
    app: tauri::AppHandle,
    directory: bool,
    initial: String,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let initial = local_path(&initial);
        let folder = if initial.is_dir() {
            initial.as_path()
        } else {
            initial.parent().unwrap_or(std::path::Path::new("."))
        };
        let dialog = app
            .dialog()
            .file()
            .set_directory(folder)
            .set_title(if directory {
                "选择数据目录"
            } else {
                "选择本机文件"
            });
        let selected = if directory {
            dialog.blocking_pick_folder()
        } else {
            dialog.blocking_pick_file()
        };
        selected
            .map(|file| {
                file.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn check_local_path(path: String) -> bool {
    local_path(&path).exists()
}

struct Shared(Arc<Mutex<Engine>>, Arc<Mutex<Engine>>, Arc<Mutex<Engine>>);

fn update_events(method: &str, result: &Result<Value, String>) -> Vec<(&'static str, Value)> {
    if method == "hosts.sample" {
        return vec![match result {
            Ok(rows) => ("desktop:hosts", rows.clone()),
            Err(error) => ("desktop:hosts-error", Value::String(error.clone())),
        }];
    }
    if method.starts_with("creditEstimates.") || method.starts_with("wakeups.") {
        return vec![("desktop:data-changed", Value::String(method.into()))];
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
            | "prices.recalculate"
            | "quotas.order.set"
            | "quotaEstimates.start"
            | "quotaEstimates.stop"
            | "quotaEstimates.restart"
    ) {
        events.push(("desktop:data-changed", Value::String(method.into())));
    }
    events
}

fn refresh_key(method: &str) -> Option<&'static str> {
    match method {
        "sources.scan" => Some("scan"),
        "quotas.refresh" => Some("quotas"),
        "prices.sync" => Some("prices"),
        "hosts.sample" => Some("hosts"),
        _ => None,
    }
}
fn refresh_feedback(
    method: &str,
    result: &Result<Value, String>,
    item_id: Option<&str>,
) -> Option<Value> {
    let key = refresh_key(method)?;
    let failures: Vec<Value> = match result {
        Ok(rows) => rows.as_array().into_iter().flatten().filter(|row| row["error"].is_string()).map(|row| {
            serde_json::json!({"id":row["id"], "accountId":row["accountId"], "name":row["name"], "error":row["error"]})
        }).collect(),
        Err(error) => vec![serde_json::json!({"id":item_id,"accountId":if key == "quotas" { item_id } else { None },"name":"刷新失败","error":error})],
    };
    let success = failures.is_empty().then(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });
    Some(
        serde_json::json!({"key":key,"busy":false,"itemId":item_id,"success":success,"failures":failures}),
    )
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
    let network_broadcast =
        method == "network.test" && params.get("proxy").is_none() && params.get("urls").is_none();
    let operation_id = params["operationId"].clone();
    let sampling = matches!(
        method.as_str(),
        "quotaEstimates.start"
            | "quotaEstimates.stop"
            | "quotaEstimates.restart"
            | "creditEstimates.start"
            | "creditEstimates.stop"
            | "creditEstimates.restart"
    );
    if sampling {
        let _ = app.emit(
            "operations:busy",
            serde_json::json!({"operationId":operation_id,"busy":true}),
        );
        let _ = app.emit(
            "operations:progress",
            serde_json::json!({"operationId":operation_id,"stage":"等待当前刷新"}),
        );
    }
    let finished_operation_id = operation_id.clone();
    let refresh_item = ["sourceId", "accountId", "hostId"]
        .iter()
        .find_map(|key| params[*key].as_str())
        .map(str::to_owned);
    if let Some(key) = refresh_key(&method) {
        let _ = app.emit(
            "desktop:refresh-status",
            serde_json::json!({"key":key,"busy":true}),
        );
    }
    let engine = if method.starts_with("network.") {
        state.2.clone()
    } else if method.starts_with("hosts.") {
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
        let progress_app = guarded_app.clone();
        engine
            .call_with_progress(
                &method,
                params,
                Box::new(move |mut progress| {
                    progress["operationId"] = operation_id.clone();
                    let _ = progress_app.emit("operations:progress", progress);
                }),
            )
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    if sampling {
        let _ = app.emit(
            "operations:busy",
            serde_json::json!({"operationId":finished_operation_id,"busy":false}),
        );
    }
    if let Some(feedback) = refresh_feedback(&changed, &result, refresh_item.as_deref()) {
        let _ = app.emit("desktop:refresh-status", feedback);
    }
    // Keep both webviews current without starting a second poller or broadcasting
    // settings that may contain credentials, proxy URLs, or shell commands.
    for (event, payload) in update_events(&changed, &result) {
        let _ = app.emit(event, payload);
    }
    if network_broadcast && let Ok(snapshot) = &result {
        let _ = app.emit("desktop:network", snapshot);
    }
    result
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let root = std::env::var_os("AIEYES_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            let runner = app
                .path()
                .resource_dir()?
                .join("binaries")
                .join(if cfg!(windows) {
                    "aieyes-core.exe"
                } else {
                    "aieyes-core"
                });
            aieyes_core::credentials::set_helper(runner.clone());
            aieyes_core::wakeups::set_runner_path(runner);
            app.manage(Shared(
                Arc::new(Mutex::new(Engine::open(&root)?)),
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
            select_local_path,
            check_local_path,
            updates::updates_info,
            updates::updates_check,
            updates::updates_panel_check,
            updates::updates_install,
            updates::updates_preferences,
            updates::updates_later,
            desktop::desktop_info,
            desktop::desktop_mode,
            desktop::desktop_panel,
            desktop::desktop_panel_pin,
            desktop::desktop_panel_page,
            desktop::desktop_action,
            desktop::desktop_panel_ready,
            desktop::desktop_detail
        ])
        .build(tauri::generate_context!())
        .expect("Aieyes startup failed");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            desktop::save(app);
            #[cfg(target_os = "windows")]
            if let Some(capsule) = app.try_state::<capsule::Capsule>() {
                capsule.stop();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn refresh_feedback_is_shared_without_leaking_settings_or_losing_failures() {
        let rows = json!([{"id":"first","error":"unavailable","private":"hidden"},{"id":"second","result":{}}]);
        let feedback = refresh_feedback("sources.scan", &Ok(rows), Some("first")).unwrap();
        assert_eq!(feedback["busy"], false);
        assert_eq!(feedback["itemId"], "first");
        assert!(feedback["success"].is_null());
        assert_eq!(feedback["failures"].as_array().unwrap().len(), 1);
        assert!(feedback.to_string().find("hidden").is_none());
        assert!(
            refresh_feedback("prices.sync", &Ok(json!({})), None).unwrap()["success"].is_number()
        );
        assert!(refresh_feedback("settings.get", &Ok(json!({"private":"hidden"})), None).is_none());
        assert_eq!(
            refresh_feedback("hosts.sample", &Err("disconnected".into()), None).unwrap()["failures"]
                [0]["error"],
            "disconnected"
        );
    }
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
