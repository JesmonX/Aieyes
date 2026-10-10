use aieyes_core::{Engine, store::home};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
};

fn main() {
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
    let mut out = io::BufWriter::new(io::stdout());
    for line in io::stdin().lock().lines() {
        let line = line?;
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
                        match engine.call_with_progress(method, v.get("params").cloned().unwrap_or(json!({})), Box::new(move |mut progress| {
                            progress["operationId"] = operation.clone();
                            let mut stdout = io::stdout().lock();
                            let _ = serde_json::to_writer(&mut stdout, &json!({"jsonrpc":"2.0","method":"operations.progress","params":progress}));
                            let _ = stdout.write_all(b"\n"); let _ = stdout.flush();
                        })) {
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
