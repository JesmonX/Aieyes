use crate::{models::Host, ssh};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
pub fn call(
    host: &Host,
    location: &super::local::Location,
    method: &str,
    params: &Value,
) -> Result<Value> {
    let request = json!({"version":1,"location":location,"method":method,"params":params});
    let command = ssh::account_command(
        host,
        &format!(
            "exec python3 - {}",
            crate::process::quote(&request.to_string())
        ),
    )?;
    let bytes = crate::process::run(
        command,
        include_bytes!("../../../../scripts/remote_codex_auth.py").to_vec(),
        std::time::Duration::from_secs(if method == "quota" {
            180
        } else if method == "status" {
            12
        } else {
            70
        }),
    )?;
    let line = bytes
        .split(|b| *b == b'\n')
        .rev()
        .find(|v| !v.is_empty())
        .context("远端账号响应缺失")?;
    let response: Value = serde_json::from_slice(line).context("远端账号响应格式无效")?;
    ensure!(response["version"] == 1, "远端账号助手版本不兼容");
    if let Some(error) = response["error"].as_str() {
        anyhow::bail!("{error}");
    }
    response
        .get("result")
        .cloned()
        .context("远端账号响应缺少结果")
}
