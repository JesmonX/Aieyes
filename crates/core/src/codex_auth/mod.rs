//! Opt-in Codex account management; credential bytes stay on their owning machine.
pub mod files;
pub mod local;
mod processes;
mod remote;
mod rpc;
use crate::{Engine, models::*, quota, store::expand};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

pub fn location(source: &Source, settings: &Settings) -> local::Location {
    local::Location {
        path: home_path(&source.path),
        binary: source.codex_binary.clone(),
        proxy: if source.host_id.is_some() {
            source.proxy.clone().unwrap_or(ProxyConfig {
                mode: "system".into(),
                url: String::new(),
            })
        } else {
            source
                .proxy
                .clone()
                .unwrap_or_else(|| settings.proxy.clone())
        },
    }
}
pub fn home_path(path: &str) -> String {
    let path = path.trim_end_matches(['/', '\\']);
    for suffix in [
        "/sessions",
        "/archived_sessions",
        "\\sessions",
        "\\archived_sessions",
    ] {
        if let Some(root) = path.strip_suffix(suffix) {
            return root.into();
        }
    }
    path.into()
}
pub fn backend(
    source: &Source,
    settings: &Settings,
    method: &str,
    params: &Value,
) -> Result<Value> {
    let account = settings.accounts.iter().find(|a| {
        a.provider == source.provider
            && (params["accountId"].as_str().is_some_and(|id| a.id == id)
                || (params["accountId"].as_str().is_none_or(str::is_empty)
                    && (a.id == source.account_id
                        || params["profileId"].as_str().is_some_and(|p| {
                            source
                                .codex_home_id
                                .as_ref()
                                .is_some_and(|home| a.has_profile(&format!("{home}:{p}")))
                        }))))
    });
    let mut configured = source.clone();
    configured.quota_pre_command = crate::accounts::pre_command(settings, account, source);
    let source = &configured;
    if let Some(host) = source.host_id.as_ref() {
        let host = settings
            .hosts
            .iter()
            .find(|h| &h.id == host && h.enabled)
            .context("目标服务器已停用或删除")?;
        remote::call(
            &quota::quota_host(host, source),
            &location(source, settings),
            method,
            params,
        )
    } else {
        local::call(&location(source, settings), method, params)
    }
}
pub fn source_for_profile<'a, 'b>(
    settings: &'a Settings,
    reference: &'b str,
) -> Result<(&'a Source, &'b str)> {
    let (home, profile) = reference.split_once(':').context("账号档案引用无效")?;
    files::id(profile)?;
    let source = settings
        .sources
        .iter()
        .find(|s| s.codex_home_id.as_deref() == Some(home) && s.enabled)
        .context("账号所在 Codex 数据源已停用或删除")?;
    Ok((source, profile))
}
pub fn read_quota(account: &Account, settings: &Settings) -> Result<QuotaSnapshot> {
    read_quota_at(account, settings, None)
}
pub fn read_quota_at(
    account: &Account,
    settings: &Settings,
    source_id: Option<&str>,
) -> Result<QuotaSnapshot> {
    let mut choices: Vec<_> = account
        .profile_refs()
        .into_iter()
        .filter_map(|r| source_for_profile(settings, r).ok())
        .map(|(s, p)| (s, Some(p)))
        .collect();
    choices.extend(
        settings
            .sources
            .iter()
            .filter(|s| {
                s.enabled
                    && s.provider == account.provider
                    && s.account_id == account.id
                    && s.codex_home_id.is_none()
            })
            .map(|s| (s, None)),
    );
    choices.retain(|(s, _)| source_id.is_none_or(|id| s.id == id));
    choices.sort_by_key(|(s, _)| {
        (
            account.quota_source_id.as_ref() != Some(&s.id),
            s.host_id.is_some(),
        )
    });
    let mut last = anyhow::anyhow!("账户所在机器已停用或未连接");
    for (source, profile) in choices {
        let mut query = source.clone();
        query.account_id = account.id.clone();
        query.name = account.name.clone();
        let result = if let Some(profile) = profile {
            backend(
                source,
                settings,
                "quota",
                &json!({"profileId":profile,"accountId":account.id}),
            )
            .and_then(|value| quota::normalize(&query, &value, "live"))
        } else {
            quota::read(&query, settings)
        };
        match result {
            Ok(q) => return Ok(q),
            Err(e) => last = e,
        }
    }
    Err(last)
}
pub fn validate_settings(s: &Settings) -> Result<()> {
    let mut homes = std::collections::HashSet::new();
    for source in &s.sources {
        if let Some(home) = &source.codex_home_id {
            files::id(home)?;
            ensure!(
                source.provider == "codex"
                    && source.account_id.is_empty()
                    && source.quota_command.is_empty(),
                "共享 Codex 来源不关联历史账号或自定义额度命令"
            );
            ensure!(homes.insert(home), "一个 Codex home 只能关联一个共享来源");
        }
    }
    let mut bindings = std::collections::HashSet::new();
    for account in &s.accounts {
        for reference in account.profile_refs() {
            ensure!(account.provider == "codex", "账号档案仅用于 Codex");
            ensure!(
                bindings.insert(reference),
                "一个账号档案只能关联一个额度账户"
            );
            let (home, profile) = reference.split_once(':').context("账号档案引用无效")?;
            files::id(profile)?;
            ensure!(
                s.sources
                    .iter()
                    .any(|v| v.codex_home_id.as_deref() == Some(home)),
                "账号档案所属 home 已删除，请先解除关联"
            );
        }
    }
    Ok(())
}
impl Engine {
    pub(crate) fn codex_auth_call(&mut self, method: &str, params: Value) -> Result<Value> {
        let method = method.strip_prefix("codexAuth.").context("账号操作无效")?;
        if method == "openUrl" {
            let url = params["url"].as_str().context("缺少登录地址")?;
            let parsed = reqwest::Url::parse(url).context("登录地址无效")?;
            ensure!(
                parsed.scheme() == "https"
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.port().is_none()
                    && matches!(
                        parsed.host_str(),
                        Some("auth.openai.com" | "chatgpt.com" | "auth0.openai.com")
                    ),
                "不支持的登录地址"
            );
            #[cfg(target_os = "macos")]
            let mut cmd = std::process::Command::new("open");
            #[cfg(target_os = "windows")]
            let mut cmd = {
                let mut c = std::process::Command::new("rundll32.exe");
                c.arg("url.dll,FileProtocolHandler");
                c
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(url);
            crate::process::run(cmd, vec![], std::time::Duration::from_secs(10))
                .map_err(|_| anyhow::anyhow!("无法打开登录页面"))?;
            return Ok(json!({"opened":true}));
        }
        let mut settings = self.store.settings()?;
        let initial = serde_json::to_value(&settings)?;
        if method == "homes.list" {
            return Ok(json!(settings.sources.iter().filter(|s|s.provider=="codex").map(|s|json!({"id":s.codex_home_id,"sourceId":s.id,"name":s.name,"hostId":s.host_id,"path":home_path(&s.path),"enabled":s.enabled})).collect::<Vec<_>>()));
        }
        let source = settings
            .sources
            .iter()
            .find(|s| {
                s.id == params["sourceId"].as_str().unwrap_or("")
                    && s.provider == "codex"
                    && s.enabled
            })
            .context("请选择已保存且启用的 Codex 来源")?
            .clone();
        if method == "enable" {
            if source.codex_home_id.is_some() {
                return Ok(json!({"enabled":true}));
            }
            ensure!(
                source.quota_command.is_empty(),
                "请先清空此来源的自定义额度命令"
            );
            ensure!(
                !self.store.wakeups()?.iter().any(|r| r
                    .deployment
                    .as_ref()
                    .is_some_and(|d| d.source.id == source.id)),
                "请先移除此来源已部署的旧唤醒任务，再启用多账号管理"
            );
            let root = home_path(&source.path);
            for other in settings.sources.iter().filter(|s| {
                s.provider == "codex" && s.id != source.id && s.host_id == source.host_id
            }) {
                let same = if source.host_id.is_some() {
                    home_path(&other.path) == root
                } else {
                    std::fs::canonicalize(expand(&home_path(&other.path)))
                        .ok()
                        .zip(std::fs::canonicalize(expand(&root)).ok())
                        .is_some_and(|(a, b)| a == b)
                };
                ensure!(
                    !same,
                    "此 home 已有其他历史来源，请先保留单一来源再启用多账号管理"
                );
            }
            let info = backend(&source, &settings, "enable", &json!({}))?;
            let home = info["homeId"]
                .as_str()
                .context("home 标识缺失")?
                .to_string();
            let s = settings
                .sources
                .iter_mut()
                .find(|s| s.id == source.id)
                .unwrap();
            s.codex_home_id = Some(home);
            s.account_id.clear();
            for a in &mut settings.accounts {
                if a.quota_source_id.as_deref() == Some(&source.id) {
                    a.quota_source_id = None;
                }
            }
            self.store
                .patch_settings(&initial, &serde_json::to_value(&settings)?, true)?;
            return Ok(json!({"enabled":true}));
        }
        ensure!(
            source.codex_home_id.is_some() || method == "inspect",
            "请先启用此来源的多账号管理"
        );
        if method == "profiles.bind" {
            let profile = params["profileId"].as_str().context("请选择账号档案")?;
            let info = backend(&source, &settings, "list", &json!({}))?;
            let p = info["profiles"]
                .as_array()
                .and_then(|rows| rows.iter().find(|p| p["id"] == profile))
                .context("账号档案不存在")?;
            let reference = format!("{}:{profile}", source.codex_home_id.as_deref().unwrap());
            let identity = p["identity"]["key"].as_str().context("无法确认账户身份")?;
            let email = p["identity"]["email"].as_str().unwrap_or("").trim();
            let requested = params["accountId"].as_str().unwrap_or("");
            let existing = settings
                .accounts
                .iter()
                .position(|a| a.has_profile(&reference));
            if let Some(index) = existing {
                ensure!(
                    requested.is_empty() || settings.accounts[index].id == requested,
                    "此登录已连接其他账户，请在账户页管理或恢复原账户"
                );
            }
            let target = existing.or_else(|| {
                settings.accounts.iter().position(|a| {
                    a.provider == "codex"
                        && if requested.is_empty() {
                            !a.archived && a.identity_key.as_deref() == Some(identity)
                        } else {
                            a.id == requested
                        }
                })
            });
            let base = serde_json::to_value(&settings)?;
            let index = if let Some(index) = target {
                let a = &settings.accounts[index];
                if let Some(key) = &a.identity_key {
                    ensure!(key == identity, "只能连接同一用户与工作区的账户");
                } else if let Some(old) = a.profile_refs().first() {
                    let (old_source, old_profile) = source_for_profile(&settings, old)?;
                    let old_info = backend(old_source, &settings, "list", &json!({}))?;
                    ensure!(
                        old_info["profiles"]
                            .as_array()
                            .and_then(|rows| rows.iter().find(|p| p["id"] == old_profile))
                            .and_then(|p| p["identity"]["key"].as_str())
                            == Some(identity),
                        "只能连接已验证为同一用户与工作区的账户"
                    );
                }
                index
            } else {
                ensure!(requested.is_empty(), "目标账户不存在");
                settings.accounts.push(Account {
                    id: format!("codex-{}", files::new_id()),
                    name: if email.is_empty() {
                        p["name"]
                            .as_str()
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Codex 账户")
                    } else {
                        email
                    }
                    .into(),
                    pending_name: email.is_empty(),
                    provider: "codex".into(),
                    ..Default::default()
                });
                settings.accounts.len() - 1
            };
            let a = &mut settings.accounts[index];
            if a.pending_name && !email.is_empty() {
                a.name = email.into();
                a.pending_name = false;
            }
            a.identity_key = Some(identity.into());
            if !a.has_profile(&reference) {
                a.connections.push(AccountConnection {
                    source_id: source.id.clone(),
                    profile_id: Some(reference.clone()),
                });
            }
            if a.quota_profile_id.is_none() {
                a.quota_profile_id = Some(reference);
            }
            let account = a.id.clone();
            self.store
                .patch_settings(&base, &serde_json::to_value(settings)?, true)?;
            return Ok(json!({"accountId":account}));
        }
        if method == "profiles.remove" {
            let reference = format!(
                "{}:{}",
                source.codex_home_id.as_deref().unwrap(),
                params["profileId"].as_str().unwrap_or("")
            );
            ensure!(
                !self
                    .store
                    .wakeups()?
                    .iter()
                    .any(|r| r.task.codex_profile_id.as_deref() == Some(&reference)),
                "请先移除引用此档案的唤醒任务"
            );
            let result = backend(&source, &settings, method, &params)?;
            for account in &mut settings.accounts {
                account
                    .connections
                    .retain(|c| c.profile_id.as_deref() != Some(&reference));
                if account.quota_profile_id.as_deref() == Some(&reference) {
                    account.quota_profile_id = account
                        .connections
                        .iter()
                        .find_map(|c| c.profile_id.clone());
                }
                if account.profile_refs().is_empty() && account.connections.is_empty() {
                    account.quota_enabled = false;
                }
            }
            self.store
                .patch_settings(&initial, &serde_json::to_value(&settings)?, true)?;
            return Ok(result);
        }
        let mut result = backend(&source, &settings, method, &params)?;
        let profile = if method == "adopt" {
            result["profile"]["id"].as_str()
        } else if method == "login.status" && result["status"] == "succeeded" {
            result["result"]["profile"]["id"].as_str()
        } else {
            None
        };
        if let Some(profile) = profile.map(str::to_owned) {
            let binding = self.codex_auth_call(
                "codexAuth.profiles.bind",
                json!({"sourceId":source.id,"profileId":profile,"accountId":params["accountId"]}),
            );
            match binding {
                Ok(bound) => {
                    result["accountId"] = bound["accountId"].clone();
                    if let Some(account) = self
                        .store
                        .settings()?
                        .accounts
                        .iter()
                        .find(|a| Some(a.id.as_str()) == bound["accountId"].as_str())
                    {
                        result["accountArchived"] = json!(account.archived);
                        result["quotaEnabled"] = json!(account.quota_enabled);
                    }
                }
                Err(error) if method == "login.status" => {
                    // Authorization already finished. Do not make clients poll a
                    // completed login forever when its requested binding fails.
                    result["status"] = json!("failed");
                    result["error"] = json!(format!(
                        "登录已保存，但账户连接失败：{error}。可读取已保存登录并重新添加到账户。"
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
