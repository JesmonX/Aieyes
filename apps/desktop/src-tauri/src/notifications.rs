use aieyes_core::{
    Engine,
    models::now,
    quota_notifications::{Kind, ResetEvent},
    quota_schedule::Poller,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

pub struct Notifications {
    stopped: Arc<AtomicBool>,
    error: Mutex<Option<String>>,
}
impl Notifications {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}
impl Drop for Notifications {
    fn drop(&mut self) {
        self.stop();
    }
}

fn permission(app: &tauri::AppHandle) -> Result<String, String> {
    app.notification()
        .permission_state()
        .map(|v| {
            match v {
                tauri::plugin::PermissionState::Granted => "granted",
                tauri::plugin::PermissionState::Denied => "denied",
                _ => "prompt",
            }
            .into()
        })
        .map_err(|e| e.to_string())
}
fn request(app: &tauri::AppHandle) -> Result<(), String> {
    if permission(app)? == "prompt" {
        app.notification()
            .request_permission()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn send(app: &tauri::AppHandle, title: &str, body: &str) -> Result<(), String> {
    if permission(app)? != "granted" {
        return Err("系统通知未授权，请在系统通知设置中允许 Aieyes".into());
    }
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}
fn content(event: &ResetEvent) -> (String, String) {
    let title = if event.kind == Kind::Unexpected {
        "订阅额度提前重置"
    } else {
        "订阅额度已重置"
    }
    .into();
    let name = aieyes_core::contracts::provider(&event.provider)
        .map(|p| p.name)
        .unwrap_or(&event.provider);
    let windows = event
        .windows
        .iter()
        .map(|w| {
            let mut text = format!("{}：剩余 {:.0}%", w.name, 100.0 - w.used_percent);
            if event.kind == Kind::Unexpected {
                // The desktop shell formats using the machine's timezone without a webview.
                let date = chrono::DateTime::from_timestamp(w.expected_reset_at, 0)
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%m/%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                text.push_str(&format!("（原预计 {date}）"));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("；");
    (title, format!("{name} · {}\n{windows}", event.name))
}
fn report(app: &tauri::AppHandle, result: Result<(), String>) {
    if let Err(error) = result {
        if let Some(state) = app.try_state::<Notifications>() {
            *state.error.lock().unwrap() = Some(error);
        }
        let _ = app.emit("desktop:notifications", ());
    }
}
#[tauri::command]
pub fn notifications_info(app: tauri::AppHandle) -> Value {
    let state = app.state::<Notifications>();
    match permission(&app) {
        Ok(permission) => {
            json!({"permission":permission,"error":state.error.lock().unwrap().clone()})
        }
        Err(error) => json!({"permission":"unknown","error":error}),
    }
}
#[tauri::command]
pub fn notifications_test(app: tauri::AppHandle) -> Result<(), String> {
    let result = request(&app)
        .and_then(|_| send(&app, "Aieyes 测试通知", "收到此通知表示系统通知可正常显示。"));
    *app.state::<Notifications>().error.lock().unwrap() = result.as_ref().err().cloned();
    let _ = app.emit("desktop:notifications", ());
    result
}

pub fn start(app: &tauri::AppHandle, root: &Path) {
    let stop = Arc::new(AtomicBool::new(false));
    let since = now();
    app.manage(Notifications {
        stopped: stop.clone(),
        error: Mutex::new(None),
    });
    // Polling and delivery have separate connections: an SSH timeout must not delay delivery.
    for polling in [true, false] {
        let app = app.clone();
        let root = root.to_owned();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut engine = None;
            let mut poller = Poller::default();
            let mut asked = false;
            while !stop.load(Ordering::Acquire) {
                if engine.is_none() {
                    engine = Engine::open(&root).ok();
                }
                if let Some(engine) = engine.as_mut() {
                    if polling && aieyes_core::operations::status()["preparing"] != true {
                        if let Ok(Some(params)) = poller.next(engine) {
                            let handle = app.clone();
                            let result = engine
                                .call_with_progress(
                                    "quotas.refresh",
                                    params,
                                    Box::new(move |p| {
                                        if p["kind"] == "quotas.refresh" {
                                            let _ = handle
                                                .emit("desktop:quotas", json!([p["row"].clone()]));
                                        }
                                    }),
                                )
                                .map_err(|e| e.to_string());
                            if let Some(feedback) =
                                super::refresh_feedback("quotas.refresh", &result, None)
                            {
                                let _ = app.emit("desktop:refresh-status", feedback);
                            }
                            if result
                                .as_ref()
                                .is_ok_and(|r| r.as_array().is_some_and(|r| !r.is_empty()))
                            {
                                let _ = app.emit("desktop:data-changed", "quotas.refresh");
                            }
                        }
                    } else if !polling {
                        if !asked
                            && engine.store.settings().is_ok_and(|s| {
                                aieyes_core::quota_notifications::monitoring_enabled(&s)
                            })
                        {
                            asked = true;
                            report(&app, request(&app));
                        }
                        match engine.call("quotaResetNotifications.take", json!({"since":since})) {
                            Ok(value) => {
                                if let Ok(events) = serde_json::from_value::<Vec<ResetEvent>>(value)
                                {
                                    for event in events {
                                        if stop.load(Ordering::Acquire) {
                                            break;
                                        }
                                        let (title, body) = content(&event);
                                        report(&app, send(&app, &title, &body));
                                    }
                                }
                            }
                            Err(error) => {
                                if aieyes_core::operations::status()["preparing"] != true {
                                    report(&app, Err(error.to_string()));
                                }
                            }
                        }
                    }
                }
                for _ in 0..if polling { 1 } else { 5 } {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn message_distinguishes_kinds_and_preserves_residual_usage() {
        let mut event = ResetEvent {
            id: "test".into(),
            account_key: "codex:a".into(),
            provider: "codex".into(),
            name: "Work".into(),
            kind: Kind::Unexpected,
            detected_at: 100,
            windows: vec![aieyes_core::quota_notifications::WindowReset {
                id: "primary".into(),
                name: "5h".into(),
                previous_used_percent: 80.0,
                used_percent: 5.0,
                expected_reset_at: 1000,
                resets_at: Some(2000),
            }],
        };
        let (title, body) = content(&event);
        assert!(title.contains("提前"));
        assert!(body.contains("95%"));
        assert!(body.contains("原预计"));
        event.kind = Kind::Natural;
        let (title, body) = content(&event);
        assert!(!title.contains("提前"));
        assert!(!body.contains("原预计"));
    }
}
