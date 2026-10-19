//! Qoder's SDK control protocol, without an Agent prompt or copied login secret.
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn read(
    source: &Source,
    settings: &Settings,
    config: &SubscriptionConfig,
) -> Result<QuotaSnapshot> {
    let root = if config.native_config_dir.trim().is_empty() {
        crate::store::expand(&crate::provider_paths::default_path("qoder", false)?)
    } else {
        crate::store::expand(&config.native_config_dir)
    };
    ensure!(
        root.is_absolute() && root.is_dir(),
        "Qoder 配置目录不存在，请先运行 qodercli login 或选择已有配置目录"
    );
    // The GUI's PATH may omit ~/.local/bin. The existing launcher quotes every
    // argument and supplies the shell PATH fallback on Unix.
    let args = [
        "--print",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--no-session-persistence",
        "--setting-sources",
        "user",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--config-dir",
        root.to_str().context("Qoder 配置目录编码无效")?,
    ];
    let mut command = crate::process::cli_command_with_pre_command(
        "qodercli",
        &args,
        if cfg!(unix) { ":" } else { "" },
        "",
    );
    command.current_dir(crate::store::home());
    network::apply_env(
        &mut command,
        source.proxy.as_ref().unwrap_or(&settings.proxy),
    );
    let frames = [
        ("initialize", "aieyes-init"),
        ("get_usage_info", "aieyes-usage"),
    ]
    .into_iter()
    .map(|(kind, id)| {
        json!({"type":"control_request","request_id":id,"request":{"type":kind,"subtype":kind}})
            .to_string()
            + "\n"
    })
    .collect::<String>();
    let output = crate::process::run(
        command,
        frames.into_bytes(),
        std::time::Duration::from_secs(45),
    )
    .map_err(|_| anyhow::anyhow!("Qoder CLI 额度查询失败；请检查安装、登录和网络"))?;
    let usage = response(&output)?;
    parse(source, config, &usage)
}

fn response(output: &[u8]) -> Result<Value> {
    ensure!(output.len() <= 2 * 1024 * 1024, "Qoder 额度响应过大");
    let mut usage = None;
    for line in output.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let Ok(frame) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if frame["type"] != "control_response" || frame["response"]["request_id"] != "aieyes-usage"
        {
            continue;
        }
        ensure!(
            usage.is_none() && frame["response"]["subtype"] == "success",
            "Qoder CLI 不支持额度查询或当前登录不可用"
        );
        usage = Some(frame["response"]["response"]["usage"].clone());
    }
    usage.context("Qoder CLI 未返回账户额度；请升级客户端并检查登录")
}

pub(super) fn check_identity(
    snapshot: &QuotaSnapshot,
    account: Option<&Account>,
    require_bound: bool,
) -> Result<()> {
    let key = &snapshot
        .identity
        .as_ref()
        .context("Qoder 未返回账户身份")?
        .key;
    let expected = account.and_then(|a| a.identity_key.as_ref());
    ensure!(
        !require_bound || expected.is_some(),
        "Qoder 账户身份尚未绑定，请重新连接 CLI 登录"
    );
    ensure!(
        expected.is_none_or(|expected| expected == key),
        "Qoder CLI 当前登录与此账户不一致；请在 CLI 切换回原账户或另建账户"
    );
    Ok(())
}

pub(super) fn apply(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let user = v["userId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .context("Qoder 未返回账户身份；会话 Credits 不能代替账户额度")?;
    q.plan = label(&v["userType"]);
    let mut any_remaining = false;
    for (field, total_field, name) in [
        ("userQuota", "total", "套餐"),
        ("addOnQuota", "total", "加购"),
        ("orgResourcePackage", "cap", "组织共享"),
    ] {
        let bucket = &v[field];
        if bucket.is_null() {
            continue;
        }
        ensure!(
            bucket["unit"]
                .as_str()
                .is_some_and(|u| u.eq_ignore_ascii_case("credits")),
            "Qoder 额度单位不支持"
        );
        let (total, used, remaining) = (
            number(&bucket[total_field])?,
            number(&bucket["used"])?,
            number(&bucket["remaining"])?,
        );
        ensure!(
            total >= 0. && used >= 0. && remaining >= 0.,
            "Qoder Credits 数值无效"
        );
        let usage = if total == 0. {
            ensure!(used == 0. && remaining == 0., "Qoder 零上限额度不一致");
            100.
        } else {
            ratio(&bucket["used"], &bucket[total_field])?
        };
        any_remaining |= remaining > 0. && bucket["available"] != false;
        window(
            q,
            field,
            &format!("{name} Credits · {used} / {total} · 剩余 {remaining}"),
            usage,
            None,
            None,
        );
    }
    ensure!(
        !q.windows.is_empty(),
        "Qoder 未返回账户额度；会话 Credits 不能代替账户额度"
    );
    q.is_available = Some(v["isQuotaExceeded"] != true && any_remaining);
    q.identity = Some(AccountIdentity {
        key: format!("qoder:{:x}", Sha256::digest(user.as_bytes())),
        subscription: q.plan.clone(),
        checked_at: now(),
        subscription_checked_at: Some(now()),
        ..Default::default()
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_response_cannot_substitute_session_or_other_request_for_account_quota() {
        let data = b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-init\",\"subtype\":\"success\",\"response\":{\"usage\":{\"wrong\":true}}}}\n{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"success\",\"response\":{\"session\":{\"total_credits\":0}}}}\n";
        let usage = response(data).unwrap();
        assert!(apply(&mut QuotaSnapshot::default(), &usage).is_err());
        assert!(response(b"{\"type\":\"control_response\",\"response\":{\"request_id\":\"aieyes-usage\",\"subtype\":\"error\",\"error\":\"PRIVATE_TOKEN\"}}\n").unwrap_err().to_string().find("PRIVATE_TOKEN").is_none());
    }
}
