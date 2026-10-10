use crate::{models::Host, ssh};
use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use serde_json::{Value, json};

pub(crate) fn script() -> Vec<u8> {
    let sources = json!({
        "__init__": include_str!("../../../../scripts/vendor/tomli/__init__.py"),
        "_types": include_str!("../../../../scripts/vendor/tomli/_types.py"),
        "_re": include_str!("../../../../scripts/vendor/tomli/_re.py"),
        "_parser": include_str!("../../../../scripts/vendor/tomli/_parser.py"),
    });
    format!(
        "import json, base64\n_aieyes_tomli_sources = json.loads(base64.b64decode('{}'))\n{}",
        base64::engine::general_purpose::STANDARD.encode(sources.to_string()),
        include_str!("../../../../scripts/remote_codex_auth.py")
    )
    .into_bytes()
}
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
        script(),
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
