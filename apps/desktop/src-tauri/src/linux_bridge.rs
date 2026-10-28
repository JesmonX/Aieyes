//! Wayland keeps the main window native. Only the pet and its panel use X11.
//! A private, generation-scoped Unix socket carries presentation and RPC messages.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::{Emitter, Listener, Manager};
const CHILD: &str = "AIEYES_X11_HELPER";
const EVENTS: &[&str] = &[
    "desktop:status",
    "desktop:panel",
    "desktop:settings",
    "desktop:data-changed",
    "desktop:quotas",
    "desktop:hosts",
    "desktop:refresh-status",
    "desktop:error",
    "desktop:invalidate",
    "desktop:network",
    "desktop:system-theme",
    "operations:busy",
    "operations:progress",
    "runtime:changed",
    "hosts:changed",
    "updates:status",
];
struct Bridge {
    writer: Arc<Mutex<Option<UnixStream>>>,
    generation: AtomicU64,
    stopped: AtomicBool,
    pending: Mutex<HashMap<u64, tauri::ipc::InvokeResolver<tauri::Wry>>>,
    sequence: AtomicU64,
    info: Mutex<Value>,
    failed: AtomicBool,
}
pub fn child() -> bool {
    std::env::var_os(CHILD).is_some()
}
pub fn needed() -> bool {
    use gtk::prelude::*;
    !child()
        && gtk::gdk::Display::default()
            .is_some_and(|display| display.type_().name().contains("Wayland"))
}
pub fn cached(app: &tauri::AppHandle) -> Option<Value> {
    app.try_state::<Bridge>().and_then(|b| {
        b.info
            .lock()
            .ok()
            .filter(|v| !v.is_null())
            .map(|v| v.clone())
    })
}
pub fn remember(app: &tauri::AppHandle, info: &Value) {
    if let Some(b) = app.try_state::<Bridge>() {
        if let Ok(mut cached) = b.info.lock() {
            *cached = info.clone();
        }
    }
}
pub fn failed(app: &tauri::AppHandle) -> bool {
    app.try_state::<Bridge>()
        .is_some_and(|b| b.failed.load(Ordering::Acquire))
}
pub fn connected(app: &tauri::AppHandle) -> bool {
    app.try_state::<Bridge>()
        .is_some_and(|b| b.writer.lock().is_ok_and(|s| s.is_some()))
}
pub fn active(app: &tauri::AppHandle) -> bool {
    !child() && app.try_state::<Bridge>().is_some()
}
fn write(app: &tauri::AppHandle, mut value: Value) -> bool {
    let Some(bridge) = app.try_state::<Bridge>() else {
        return false;
    };
    value["generation"] = bridge.generation.load(Ordering::Acquire).into();
    let Ok(mut writer) = bridge.writer.lock() else {
        return false;
    };
    let Some(writer) = writer.as_mut() else {
        return false;
    };
    serde_json::to_vec(&value).ok().is_some_and(|mut bytes| {
        bytes.push(b'\n');
        writer.write_all(&bytes).is_ok()
    })
}
pub fn send(app: &tauri::AppHandle, name: &str, payload: Value) -> bool {
    write(app, json!({"type":"control","name":name,"payload":payload}))
}
pub fn request(app: &tauri::AppHandle, command: &str, args: Value) {
    let _ = write(app, json!({"type":"request","command":command,"args":args}));
}
pub fn forward(invoke: tauri::ipc::Invoke) -> Result<(), tauri::ipc::Invoke> {
    if !child() {
        return Err(invoke);
    }
    // File pickers belong to the requesting X11 window. Renderer readiness is local.
    if matches!(
        invoke.message.command(),
        "desktop_panel_ready" | "select_local_path" | "check_local_path"
    ) {
        return Err(invoke);
    }
    let app = invoke.message.webview().app_handle().clone();
    let Some(bridge) = app.try_state::<Bridge>() else {
        invoke.resolver.reject("辅助进程连接不可用");
        return Ok(());
    };
    let id = bridge.sequence.fetch_add(1, Ordering::Relaxed);
    let args = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(value) => value.clone(),
        _ => Value::Null,
    };
    let command = invoke.message.command().to_owned();
    {
        let mut pending = bridge.pending.lock().unwrap();
        if pending.len() >= 128 {
            invoke.resolver.reject("辅助进程请求过多，请稍后重试");
            return Ok(());
        }
        pending.insert(id, invoke.resolver);
    }
    if !write(
        &app,
        json!({"type":"request","id":id,"command":command,"args":args}),
    ) {
        if let Some(resolver) = bridge.pending.lock().unwrap().remove(&id) {
            resolver.reject("辅助进程连接已断开");
        }
    }
    Ok(())
}
// Called before dispatching a native pet/menu action in the child.
pub fn action(app: &tauri::AppHandle, action: &str) -> bool {
    if !child() || action == "menu" {
        return false;
    }
    request(app, "desktop_action", json!({"action":action}));
    true
}
fn install(app: &tauri::AppHandle, generation: u64, stream: Option<UnixStream>) {
    app.manage(Bridge {
        writer: Arc::new(Mutex::new(stream)),
        generation: AtomicU64::new(generation),
        stopped: AtomicBool::new(false),
        pending: Mutex::new(HashMap::new()),
        sequence: AtomicU64::new(1),
        info: Mutex::new(Value::Null),
        failed: AtomicBool::new(false),
    });
}
pub fn connect(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var(CHILD)?;
    let generation = std::env::var("AIEYES_HELPER_GENERATION")?.parse()?;
    let stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    install(app, generation, Some(stream.try_clone()?));
    reader(app.clone(), stream, generation, true);
    Ok(())
}
pub fn start(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    install(app, 0, None);
    for name in EVENTS {
        let handle = app.clone();
        let name = *name;
        app.listen(name, move |event| {
            let payload = serde_json::from_str(event.payload()).unwrap_or(Value::Null);
            let _ = write(
                &handle,
                json!({"type":"event","name":name,"payload":payload}),
            );
        });
    }
    let handle = app.clone();
    app.listen("linux:helper-reply", move |event| {
        if let Ok(mut value) = serde_json::from_str::<Value>(event.payload()) {
            if value["generation"].as_u64()
                != Some(handle.state::<Bridge>().generation.load(Ordering::Acquire))
            {
                return;
            }
            value["type"] = "response".into();
            let _ = write(&handle, value);
        }
    });
    let handle = app.clone();
    std::thread::spawn(move || {
        for attempt in 0..2 {
            if handle.state::<Bridge>().stopped.load(Ordering::Acquire) {
                return;
            }
            let generation = handle
                .state::<Bridge>()
                .generation
                .fetch_add(1, Ordering::AcqRel)
                + 1;
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                if std::env::var_os("DISPLAY").is_none() || x11rb::connect(None).is_err() {
                    return Err("XWayland 不可用".into());
                }
                let directory = tempfile::Builder::new().prefix("aieyes-x11-").tempdir()?;
                std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
                let socket = directory.path().join("bridge.sock");
                let listener = UnixListener::bind(&socket)?;
                listener.set_nonblocking(true)?;
                let mut process = std::process::Command::new(std::env::current_exe()?)
                    .env(CHILD, &socket)
                    .env("AIEYES_HELPER_GENERATION", generation.to_string())
                    .env("GDK_BACKEND", "x11")
                    .env_remove("TAURI_WEBVIEW_AUTOMATION")
                    .env_remove("WEBKIT_INSPECTOR_SERVER")
                    .spawn()?;
                let connected = (0..150).find_map(|_| {
                    if let Ok((stream, _)) = listener.accept() {
                        return Some(stream);
                    }
                    if process.try_wait().ok().flatten().is_some() {
                        return None;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    None
                });
                let Some(stream) = connected else {
                    let _ = process.kill();
                    let _ = process.wait();
                    return Err("辅助进程启动超时".into());
                };
                stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
                *handle.state::<Bridge>().writer.lock().unwrap() = Some(stream.try_clone()?);
                if let Ok(info) = crate::desktop::desktop_info(handle.clone()) {
                    send(&handle, "status", info);
                }
                reader(handle.clone(), stream, generation, false);
                loop {
                    if process.try_wait()?.is_some() {
                        break;
                    }
                    if handle.state::<Bridge>().stopped.load(Ordering::Acquire) {
                        let _ = process.kill();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = process.wait();
                *handle.state::<Bridge>().writer.lock().unwrap() = None;
                Ok(())
            })();
            if handle.state::<Bridge>().stopped.load(Ordering::Acquire) {
                return;
            }
            eprintln!(
                "X11 helper attempt {} ended: {:?}",
                attempt + 1,
                result.err()
            );
        }
        handle
            .state::<Bridge>()
            .failed
            .store(true, Ordering::Release);
        let owner = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let _ = crate::desktop::activate_main(&owner);
            let _ = owner.emit("desktop:error", "悬浮入口不可用，已返回主窗口");
        });
    });
    Ok(())
}
fn reader(app: tauri::AppHandle, stream: UnixStream, generation: u64, is_child: bool) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut bytes = Vec::new();
            // A bounded frame prevents a corrupt peer from growing memory indefinitely.
            let read =
                std::io::Read::take(&mut reader, 32 * 1024 * 1024).read_until(b'\n', &mut bytes);
            if !matches!(read,Ok(n) if n>0) || bytes.last() != Some(&b'\n') {
                break;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                break;
            };
            if value["generation"].as_u64() != Some(generation)
                || app.state::<Bridge>().generation.load(Ordering::Acquire) != generation
            {
                continue;
            }
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                // The callback can wait in GTK's queue across a helper restart.
                if handle.state::<Bridge>().generation.load(Ordering::Acquire) != generation {
                    return;
                }
                if is_child {
                    match value["type"].as_str() {
                        Some("response") => {
                            if let Some(id) = value["id"].as_u64() {
                                let resolver = handle.state::<Bridge>().pending.lock().unwrap().remove(&id);
                                if let Some(resolver) = resolver {
                                    if let Some(error) = value.get("error") {
                                        resolver.reject(error.clone());
                                    } else {
                                        resolver.resolve(value["value"].clone());
                                    }
                                }
                            }
                        }
                        Some("event") => {
                            let name = value["name"].as_str().unwrap_or("");
                            if matches!(name, "desktop:status" | "desktop:panel") {
                                crate::desktop::helper_status(&handle, value["payload"].clone());
                            }
                            let _ = handle.emit(name, &value["payload"]);
                        }
                        Some("control") => crate::desktop::helper_control(
                            &handle, value["name"].as_str().unwrap_or(""), value["payload"].clone(),
                        ),
                        _ => {}
                    }
                } else if value["type"] == "request" {
                    let command = value["command"].as_str().unwrap_or("");
                    if command == "helper_pet_moved" {
                        crate::desktop::pet_moved(
                            &handle,
                            value["args"]["x"].as_i64().unwrap_or(0) as i32,
                            value["args"]["y"].as_i64().unwrap_or(0) as i32,
                        );
                        return;
                    }
                    // Reuse the main dispatcher's configuration lanes, cancellation
                    // and progress events. The helper owns no core connection.
                    if let Some(main) = handle.get_webview_window("main") {
                        let command = serde_json::to_string(command).unwrap();
                        let args = value["args"].to_string();
                        let id = value["id"].clone();
                        let script = format!(
                            "(async()=>{{for(let n=0;n<100&&!window.__TAURI__?.core;n++)await new Promise(r=>setTimeout(r,100));let reply={{id:{id},generation:{generation}}};try{{reply.value=await window.__TAURI__.core.invoke({command},{args})}}catch(e){{reply.error=String(e)}}await window.__TAURI__.event.emit('linux:helper-reply',reply)}})()"
                        );
                        let _ = main.eval(script);
                    }
                }
            });
        }
        if is_child {
            app.exit(0);
        }
    });
}
pub fn stop(app: &tauri::AppHandle) {
    if let Some(bridge) = app.try_state::<Bridge>() {
        bridge.stopped.store(true, Ordering::Release);
        if let Some(stream) = bridge.writer.lock().unwrap().take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}
