use aieyes_core::{Engine, store::home};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};

fn main() {
    if aieyes_core::session_hooks::run_if_requested() {
        return;
    }
    if let Err(e) = run() {
        eprintln!("Aieyes: {e}");
        std::process::exit(1);
    }
}
fn run() -> anyhow::Result<()> {
    if aieyes_core::credentials::askpass()? {
        return Ok(());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--codex-api-key") {
        return aieyes_core::codex_api::key(
            std::path::Path::new(
                args.get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("Missing API root"))?,
            ),
            args.get(i + 2)
                .ok_or_else(|| anyhow::anyhow!("Missing API id"))?,
            args.get(i + 3)
                .ok_or_else(|| anyhow::anyhow!("Missing API revision"))?,
        );
    }
    if let Some(i) = args.iter().position(|a| a == "--wake-service") {
        let root = args
            .get(i + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing runner root"))?;
        return aieyes_core::wakeups::runner::service(std::path::Path::new(root));
    }
    if let Some(i) = args.iter().position(|a| a == "--wake-runner") {
        let root = args
            .get(i + 1)
            .ok_or_else(|| anyhow::anyhow!("Missing runner root"))?;
        return aieyes_core::wakeups::runner::run(
            std::path::Path::new(root),
            args.get(i + 2).map(String::as_str),
        );
    }
    if let Some(i) = args.iter().position(|a| a == "--codex-api-service") {
        return aieyes_core::codex_api::gateway::serve(std::path::Path::new(
            args.get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("Missing API root"))?,
        ));
    }
    if let Some(i) = args.iter().position(|a| a == "--codex-api-token") {
        return aieyes_core::codex_api::token(
            std::path::Path::new(
                args.get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("Missing API root"))?,
            ),
            args.get(i + 2)
                .ok_or_else(|| anyhow::anyhow!("Missing API id"))?,
        );
    }
    let root = args
        .windows(2)
        .find(|a| a[0] == "--data-dir")
        .map(|a| PathBuf::from(&a[1]))
        .or_else(|| std::env::var_os("AIEYES_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(default_root);
    if args.iter().any(|arg| arg == "--update-transport") {
        return aieyes_core::update_transport::serve(&root);
    }
    aieyes_core::process::install_query_shutdown_handler();
    let mut engine = Engine::open(&root)?;
    if let Some(i) = args.iter().position(|a| a == "--call") {
        let method = args
            .get(i + 1)
            .ok_or_else(|| anyhow::anyhow!("--call requires a method"))?;
        let params = args
            .get(i + 2)
            .map(|s| serde_json::from_str(s))
            .transpose()?
            .unwrap_or(json!({}));
        println!(
            "{}",
            serde_json::to_string_pretty(&engine.call(method, params)?)?
        );
        return Ok(());
    }
    engine.set_host_event_sink(|state| {
        let mut out = io::stdout().lock();
        let _ = serde_json::to_writer(
            &mut out,
            &json!({"jsonrpc":"2.0","method":"hosts.changed","params":state}),
        );
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    });
    // Read controls independently of the worker. A blocked SSH/SQLite operation must
    // not prevent cancellation or status polling during update preparation.
    let (requests, incoming) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let mut operation = None;
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let result = match v["method"].as_str() {
                    Some("operations.prepareUpdate") => Some(aieyes_core::operations::prepare()),
                    Some("operations.resume") => Some(aieyes_core::operations::resume()),
                    Some("operations.status") => Some(aieyes_core::operations::status()),
                    _ => None,
                };
                if let Some(result) = result {
                    if let Some(id) = v.get("id") {
                        let mut out = io::stdout().lock();
                        let _ = serde_json::to_writer(
                            &mut out,
                            &json!({"jsonrpc":"2.0","id":id,"result":result}),
                        );
                        let _ = out.write_all(b"\n");
                        let _ = out.flush();
                    }
                    continue;
                }
                if let Some(method) = v["method"].as_str() {
                    match aieyes_core::operations::enter(method) {
                        Ok(_scope) => operation = aieyes_core::operations::context(),
                        Err(error) => {
                            if let Some(id) = v.get("id") {
                                let mut out = io::stdout().lock();
                                let _ = serde_json::to_writer(
                                    &mut out,
                                    &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}}),
                                );
                                let _ = out.write_all(b"\n");
                                let _ = out.flush();
                            }
                            continue;
                        }
                    }
                }
            }
            if requests.send((line, operation)).is_err() {
                break;
            }
        }
    });
    for (line, operation) in incoming {
        let _operation = aieyes_core::operations::inherit(operation);
        if line.trim().is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<Value>(&line);
        let reply = match parsed {
            Ok(v) => {
                let id = v.get("id").cloned().unwrap_or(Value::Null);
                match v["method"].as_str() {
                    Some(method) => {
                        let operation = v["params"]["operationId"].clone();
                        let request_id = id.clone();
                        let result = engine.call_with_progress(method, v.get("params").cloned().unwrap_or(json!({})), Box::new(move |mut progress| {
                            progress["operationId"] = operation.clone();
                            progress["requestId"] = request_id.clone();
                            let mut stdout = io::stdout().lock();
                            let _ = serde_json::to_writer(&mut stdout, &json!({"jsonrpc":"2.0","method":"operations.progress","params":progress}));
                            let _ = stdout.write_all(b"\n"); let _ = stdout.flush();
                        }));
                        if let Some(params) = engine.take_runtime_change() {
                            let mut stdout = io::stdout().lock();
                            let _ = serde_json::to_writer(
                                &mut stdout,
                                &json!({"jsonrpc":"2.0","method":"runtime.changed","params":params}),
                            );
                            let _ = stdout.write_all(b"\n");
                            let _ = stdout.flush();
                        }
                        match result {
                            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                            Err(e) => {
                                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":e.to_string()}})
                            }
                        }
                    }
                    None => {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"请求缺少 method"}})
                    }
                }
            }
            Err(_) => {
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"JSON 格式不正确"}})
            }
        };
        let mut out = io::stdout().lock();
        serde_json::to_writer(&mut out, &reply)?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    Ok(())
}
fn default_root() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Aieyes")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("Aieyes")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/share"))
            .join("aieyes")
    }
}
