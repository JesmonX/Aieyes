use super::{
    files::{self, Profile},
    processes,
    rpc::Rpc,
};
use crate::{
    models::{ProxyConfig, now},
    store::expand,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub path: String,
    pub binary: String,
    pub proxy: ProxyConfig,
}
impl Location {
    pub fn root(&self) -> Result<PathBuf> {
        let root = expand(&self.path);
        ensure!(root.is_dir(), "Codex 目录不存在");
        files::safe(&root)?;
        std::fs::canonicalize(root).context("无法读取 Codex 目录")
    }
}
fn layout(root: &Path, profile: &str, kind: &str) -> Result<PathBuf> {
    let dir = root
        .join(".aieyes/codex")
        .join(files::id(profile)?)
        .join(kind);
    files::private_dir(&dir)?;
    Ok(dir)
}
fn current(root: &Path) -> Option<String> {
    files::read(&root.join("auth.json"))
        .ok()
        .and_then(|v| files::identity(&v).ok())
        .map(|v| v.key)
}
fn authority(root: &Path, profile: &Profile) -> Result<PathBuf> {
    let path = root.join("auth.json");
    if path.exists() {
        let auth = files::read(&path)?;
        if auth["auth_mode"] != "apikey" && files::identity(&auth)?.key == profile.identity.key {
            return Ok(path);
        }
    }
    Ok(files::profile_path(root, &profile.id)?.join("auth.json"))
}
fn read_authority(root: &Path, profile: &Profile) -> Result<Value> {
    let value = files::read(&authority(root, profile)?)?;
    ensure!(
        files::identity(&value)?.key == profile.identity.key,
        "账号身份改变，已停止操作"
    );
    Ok(value)
}
fn config(root: &Path) -> Result<toml::Value> {
    let path = root.join("config.toml");
    files::safe(&path)?;
    let text = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => anyhow::bail!("无法读取 Codex 配置"),
    };
    toml::from_str(&text).map_err(|_| anyhow::anyhow!("Codex 配置格式无效"))
}
fn mode(root: &Path) -> Result<String> {
    let c = config(root)?;
    ensure!(
        c.get("profile").is_none(),
        "请先移除 home 配置的默认 profile，避免认证策略歧义"
    );
    Ok(c.get("cli_auth_credentials_store")
        .and_then(|v| v.as_str())
        .unwrap_or("file")
        .into())
}
fn policy(root: &Path, auth: &Value) -> Result<()> {
    let c = config(root)?;
    ensure!(
        c.get("forced_login_method")
            .and_then(|v| v.as_str())
            .is_none_or(|v| v == "chatgpt"),
        "此 home 限制了登录方式"
    );
    let identity = files::identity(auth)?;
    ensure!(
        c.get("forced_chatgpt_workspace_id")
            .and_then(|v| v.as_str())
            .is_none_or(|v| v == identity.workspace),
        "此 home 限制了 ChatGPT 工作区"
    );
    Ok(())
}
fn file_mode(root: &Path) -> Result<()> {
    ensure!(
        mode(root)? == "file",
        "此 home 使用 keyring/auto 或其他认证存储，不能直接切换 auth.json；请先自行配置文件登录"
    );
    Ok(())
}
fn inspect(loc: &Location, root: &Path) -> Result<Value> {
    let active = current(root);
    let rows = files::profiles(root)?.into_iter().map(|p| {
        let is_current = active.as_deref() == Some(&p.identity.key);
        json!({"id":p.id,"name":p.name,"identity":p.identity,"updatedAt":p.updated_at,"current":is_current})
    }).collect::<Vec<_>>();
    let mut cmd = crate::process::cli_command(&crate::quota::resolve_codex(&loc.binary));
    cmd.arg("--version");
    let version = crate::process::run(cmd, vec![], Duration::from_secs(5))
        .ok()
        .map(|b| {
            String::from_utf8_lossy(&b)
                .trim()
                .chars()
                .take(80)
                .collect::<String>()
        });
    Ok(
        json!({"protocolVersion":1,"path":root,"version":version,"storageMode":mode(root)?,"currentIdentity":active,"profiles":rows,"processes":processes::list(root)?}),
    )
}
fn status(root: &Path) -> Result<Value> {
    let mode = mode(root)?;
    let path = root.join("auth.json");
    let active = if path.exists() {
        Some(files::identity(&files::read(&path)?)?.key)
    } else {
        None
    };
    let rows = files::profiles(root)?
        .into_iter()
        .map(|p| {
            let credential = read_authority(root, &p).is_ok();
            json!({"id":p.id,"identityKey":p.identity.key,"credential":credential})
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"storageMode":mode,"currentIdentity":active,"currentCredential":active.is_some(),"profiles":rows}),
    )
}
fn snapshot(root: &Path) -> Value {
    json!({"auth": files::read(&root.join("auth.json")).ok().map(|v| files::revision(&v)),"profiles":files::profiles(root).ok().map(|v| v.into_iter().map(|p| json!({"id":p.id,"auth":files::read(&files::profile_path(root,&p.id).unwrap().join("auth.json")).ok().map(|v| files::revision(&v))})).collect::<Vec<_>>())})
}
fn operation(root: &Path, id: &str) -> Result<PathBuf> {
    Ok(root
        .join(".aieyes/state")
        .join(format!("{}.json", files::id(id)?)))
}
fn update_operation(root: &Path, id: &str, value: &Value) -> Result<()> {
    files::atomic(&operation(root, id)?, value)
}
fn cancelled(root: &Path, id: &str) -> bool {
    root.join(".aieyes/state")
        .join(format!("{id}.cancel"))
        .exists()
}

/// Recover a refreshed credential before ever seeding a managed directory again.
pub(super) fn recover(root: &Path) -> Result<()> {
    let journal = root.join(".aieyes/state/refresh.json");
    if !journal.exists() {
        return Ok(());
    }
    let record = files::read(&journal)?;
    let p = files::profile(
        root,
        record["profileId"].as_str().context("续期恢复记录无效")?,
    )?;
    let stage = layout(root, &p.id, "managed")?.join("auth.json");
    let next = files::read(&stage)?;
    ensure!(
        files::identity(&next)?.key == p.identity.key,
        "续期恢复身份冲突，请重新登录"
    );
    let target = if record["current"] == true {
        root.join("auth.json")
    } else {
        files::profile_path(root, &p.id)?.join("auth.json")
    };
    let original = files::read(&target)?;
    let original_rev = files::revision(&original);
    let next_rev = files::revision(&next);
    ensure!(
        original_rev == record["revision"].as_str().unwrap_or("") || original_rev == next_rev,
        "凭据在续期期间被外部修改；已保留两份结果，请重新登录解除冲突"
    );
    if record["current"] == true {
        ensure!(
            processes::list(root)?.is_empty(),
            "续期待恢复，请先关闭此位置的 Codex"
        );
    }
    files::atomic(&target, &next)?;
    files::save_profile(root, &p.id, &p.name, &next)?;
    std::fs::remove_file(journal)?;
    // Refresh credentials are never seeded into the read-only query home.
    Ok(())
}
fn renew(loc: &Location, root: &Path, p: &Profile) -> Result<Value> {
    recover(root)?;
    let target = authority(root, p)?;
    let is_current = target == root.join("auth.json");
    if is_current {
        ensure!(
            processes::list(root)?.is_empty(),
            "当前账号由运行中的 Codex 续期；暂时显示缓存额度"
        );
    }
    let auth = read_authority(root, p)?;
    let managed = layout(root, &p.id, "managed")?;
    files::atomic(&managed.join("auth.json"), &auth)?;
    files::atomic(
        &root.join(".aieyes/state/refresh.json"),
        &json!({"profileId":p.id,"current":is_current,"revision":files::revision(&auth)}),
    )?;
    let result = (|| -> Result<()> {
        let mut rpc = Rpc::start(&loc.binary, &managed, &loc.proxy, true)?;
        rpc.call("account/read", json!({"refreshToken":true}), &mut |_| {
            Ok(None)
        })?;
        Ok(())
    })();
    // The refresh could succeed even when its RPC response was lost.
    recover(root)?;
    result?;
    read_authority(root, p)
}
fn quota(loc: &Location, root: &Path, profile: &str) -> Result<Value> {
    let _lock = files::lock(root)?;
    file_mode(root)?;
    recover(root)?;
    let p = files::profile(root, profile)?;
    let active_before = current(root);
    let auth = read_authority(root, &p)?;
    let query = layout(root, profile, "query")?;
    let attempt = |auth: &Value| -> Result<Value> {
        let mut rpc = Rpc::start(&loc.binary, &query, &loc.proxy, false)?;
        rpc.login_external(auth)?;
        let quota = rpc.quota(auth, &mut || read_authority(root, &p))?;
        ensure!(
            current(root) == active_before,
            "日常账号已改变，已丢弃旧查询结果"
        );
        Ok(quota)
    };
    match attempt(&auth) {
        Ok(value) => Ok(value),
        Err(first) => {
            let latest = read_authority(root, &p)?;
            if files::revision(&latest) != files::revision(&auth) {
                return attempt(&latest);
            }
            // No copied refresh token may race the currently running CLI.
            if active_before.as_deref() == Some(&p.identity.key)
                && !processes::list(root)?.is_empty()
            {
                return Err(first);
            }
            if !["认证已失效", "接口不兼容"]
                .iter()
                .any(|v| first.to_string().contains(v))
            {
                return Err(first);
            }
            let fresh = renew(loc, root, &p)?;
            match attempt(&fresh) {
                Ok(value) => Ok(value),
                Err(second) => {
                    if !second.to_string().contains("接口不兼容") {
                        return Err(second);
                    }
                    // Old CLIs can still query an inactive credential under our exclusive lock.
                    ensure!(
                        current(root).as_deref() != Some(&p.identity.key)
                            || processes::list(root)?.is_empty(),
                        "当前账号暂时显示缓存额度"
                    );
                    let managed = layout(root, profile, "managed")?;
                    files::atomic(&managed.join("auth.json"), &fresh)?;
                    files::atomic(
                        &root.join(".aieyes/state/refresh.json"),
                        &json!({"profileId":p.id,"current":current(root).as_deref()==Some(&p.identity.key),"revision":files::revision(&fresh)}),
                    )?;
                    let result = (|| -> Result<Value> {
                        let mut rpc = Rpc::start(&loc.binary, &managed, &loc.proxy, true)?;
                        rpc.call("account/rateLimits/read", Value::Null, &mut |_| Ok(None))
                    })();
                    recover(root)?;
                    result
                }
            }
        }
    }
}
// The worker takes ownership of the location, login snapshot and held file lock.
#[allow(clippy::too_many_arguments)]
fn login(
    loc: Location,
    root: PathBuf,
    op: String,
    name: String,
    device: bool,
    replacement: Option<Profile>,
    before: Value,
    _lock: crate::file_lock::FileLock,
) {
    let profile = files::new_id();
    let result = (|| -> Result<Value> {
        let managed = layout(&root, &profile, "managed")?;
        let mut rpc = Rpc::start(&loc.binary, &managed, &loc.proxy, true)?;
        let auth = rpc.call(
            "account/login/start",
            json!({"type":if device {"chatgptDeviceCode"} else {"chatgpt"}}),
            &mut |_| Ok(None),
        )?;
        let url = auth["authUrl"]
            .as_str()
            .or(auth["verificationUrl"].as_str())
            .context("此 CLI 未提供登录地址，请升级 CLI")?;
        let parsed = reqwest::Url::parse(url).context("登录地址无效")?;
        ensure!(
            parsed.scheme() == "https"
                && parsed.host_str().is_some_and(|v| v == "auth.openai.com"
                    || v == "chatgpt.com"
                    || v == "auth0.openai.com"),
            "CLI 返回了不受支持的登录地址"
        );
        update_operation(
            &root,
            &op,
            &json!({"operationId":op,"status":"awaitingAuthorization","authUrl":url,"userCode":auth["userCode"],"startedAt":now()}),
        )?;
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if cancelled(&root, &op) {
                let _ = rpc.call(
                    "account/login/cancel",
                    json!({"loginId":auth["loginId"]}),
                    &mut |_| Ok(None),
                );
                anyhow::bail!("登录已取消");
            }
            ensure!(Instant::now() < deadline, "登录等待超时，请重试");
            match rpc.receive(Duration::from_millis(250)) {
                Ok(v)
                    if v["method"] == "account/login/completed"
                        && v["params"]["loginId"] == auth["loginId"] =>
                {
                    ensure!(v["params"]["success"] == true, "登录未成功，请重试");
                    break;
                }
                Ok(_) => {}
                Err(e) if e.to_string().contains("超时") => {}
                Err(e) => return Err(e),
            }
        }
        let account = rpc.call("account/read", json!({"refreshToken":false}), &mut |_| {
            Ok(None)
        })?;
        ensure!(account["account"]["type"] == "chatgpt", "需要 ChatGPT 登录");
        drop(rpc);
        let auth = files::read(&managed.join("auth.json"))?;
        let identity = files::identity(&auth)?;
        policy(&root, &auth)?;
        {
            let mut verify = Rpc::start(
                &loc.binary,
                &layout(&root, &profile, "query")?,
                &loc.proxy,
                false,
            )?;
            verify.login_external(&auth)?;
            verify.quota(&auth, &mut || Ok(auth.clone()))?;
        }
        let p = if let Some(p) = replacement {
            ensure!(
                p.identity.key == identity.key,
                "重新登录必须使用原用户与工作区"
            );
            ensure!(
                snapshot(&root) == before,
                "授权期间凭据已改变，原凭据已保留，请重试"
            );
            if current(&root).as_deref() == Some(&p.identity.key) {
                ensure!(
                    processes::list(&root)?.is_empty(),
                    "请关闭日常 Codex 后重新授权"
                );
            }
            let original = read_authority(&root, &p)?;
            files::atomic(&layout(&root, &p.id, "managed")?.join("auth.json"), &auth)?;
            files::atomic(
                &root.join(".aieyes/state/refresh.json"),
                &json!({"profileId":p.id,"current":current(&root).as_deref()==Some(&p.identity.key),"revision":files::revision(&original)}),
            )?;
            recover(&root)?;
            files::save_profile(&root, &p.id, &name, &auth)?
        } else {
            ensure!(
                !files::profiles(&root)?
                    .iter()
                    .any(|p| p.identity.key == identity.key),
                "此用户与工作区已有档案，请使用重新登录"
            );
            files::save_profile(&root, &profile, &name, &auth)?
        };
        Ok(json!({"profile":p}))
    })();
    let stage = root.join(".aieyes/codex").join(&profile);
    if files::safe(&stage).is_ok() {
        let _ = std::fs::remove_dir_all(stage);
    }
    let final_state = match result {
        Ok(v) => json!({"operationId":op,"status":"succeeded","result":v,"finishedAt":now()}),
        Err(e) => {
            json!({"operationId":op,"status":if cancelled(&root,&op){"cancelled"}else{"failed"},"error":e.to_string(),"finishedAt":now()})
        }
    };
    let _ = update_operation(&root, &op, &final_state);
}
pub fn call(loc: &Location, method: &str, params: &Value) -> Result<Value> {
    let root = loc.root()?;
    match method {
        "enable" => {
            let _lock = files::lock(&root)?;
            file_mode(&root)?;
            let marker = root.join(".aieyes/state/home.json");
            if marker.exists() {
                return files::read(&marker);
            }
            let info = json!({"homeId":files::new_id(),"version":1});
            files::atomic(&marker, &info)?;
            Ok(info)
        }
        "status" => status(&root),
        "inspect" | "list" => inspect(loc, &root),
        "model.list" => {
            let _lock = files::lock(&root)?;
            file_mode(&root)?;
            recover(&root)?;
            let p = files::profile(
                &root,
                params["profileId"].as_str().context("请选择账号档案")?,
            )?;
            let auth = read_authority(&root, &p)?;
            let path = layout(&root, &p.id, "query")?;
            let mut rpc = Rpc::start(&loc.binary, &path, &loc.proxy, false)?;
            let mut info = rpc.login_external(&auth)?;
            info["models"] = rpc.call("model/list", json!({"includeHidden":false}), &mut |_| {
                Ok(None)
            })?;
            Ok(info)
        }
        "quota" => quota(
            loc,
            &root,
            params["profileId"].as_str().context("请选择账号档案")?,
        ),
        "login.status" => {
            let op = params["operationId"].as_str().context("缺少操作标识")?;
            let mut state = files::read(&operation(&root, op)?)?;
            if matches!(
                state["status"].as_str(),
                Some("running" | "awaitingAuthorization")
            ) && files::lock(&root).is_ok()
            {
                state = json!({"operationId":op,"status":"failed","error":"登录进程已退出，请重新登录"});
                update_operation(&root, op, &state)?;
            }
            Ok(state)
        }
        "login.cancel" => {
            let op = params["operationId"].as_str().context("缺少操作标识")?;
            files::read(&operation(&root, op)?)?;
            files::atomic(
                &root
                    .join(".aieyes/state")
                    .join(format!("{}.cancel", files::id(op)?)),
                &json!(true),
            )?;
            Ok(json!({"cancelRequested":true}))
        }
        "login.start" => {
            let lock = files::lock(&root)?;
            file_mode(&root)?;
            let replacement = params["profileId"]
                .as_str()
                .map(|id| files::profile(&root, id))
                .transpose()?;
            let journal = root.join(".aieyes/state/refresh.json");
            if let Some(p) = &replacement {
                if current(&root).as_deref() == Some(&p.identity.key) {
                    ensure!(
                        processes::list(&root)?.is_empty(),
                        "请先关闭日常 Codex，再重新登录"
                    );
                }
                if journal.exists() {
                    ensure!(
                        files::read(&journal)?["profileId"] == p.id,
                        "请先重新登录发生续期冲突的账号"
                    );
                }
            } else {
                recover(&root)?;
            }
            let before = snapshot(&root);
            let name = params["name"].as_str().unwrap_or("").trim().to_string();
            ensure!(name.chars().count() <= 100, "账号名称不能超过 100 字");
            let op = files::new_id();
            update_operation(
                &root,
                &op,
                &json!({"operationId":op,"status":"running","startedAt":now()}),
            )?;
            let copy = op.clone();
            let loc = loc.clone();
            let device = params["deviceCode"] == true;
            std::thread::spawn(move || {
                login(loc, root, copy, name, device, replacement, before, lock)
            });
            Ok(json!({"operationId":op,"status":"running"}))
        }
        "adopt" => {
            let _lock = files::lock(&root)?;
            file_mode(&root)?;
            recover(&root)?;
            let auth = files::read(&root.join("auth.json"))?;
            let ident = files::identity(&auth)?;
            if let Some(existing) = files::profiles(&root)?
                .into_iter()
                .find(|p| p.identity.key == ident.key)
            {
                return Ok(json!({"profile":existing}));
            }
            let query = layout(&root, "verify", "query")?;
            let mut rpc = Rpc::start(&loc.binary, &query, &loc.proxy, false)?;
            rpc.login_external(&auth)?;
            policy(&root, &auth)?;
            rpc.quota(&auth, &mut || Ok(auth.clone()))?;
            ensure!(
                files::revision(&files::read(&root.join("auth.json"))?) == files::revision(&auth),
                "登录状态已改变，请重新读取"
            );
            let p = files::save_profile(
                &root,
                &files::new_id(),
                params["name"].as_str().unwrap_or(""),
                &auth,
            )?;
            Ok(json!({"profile":p}))
        }
        "switch.prepare" => {
            let _lock = files::lock(&root)?;
            file_mode(&root)?;
            recover(&root)?;
            let p = files::profile(
                &root,
                params["profileId"].as_str().context("请选择目标账号")?,
            )?;
            let op = files::new_id();
            let running = processes::list(&root)?;
            let record = json!({"operationId":op,"status":"prepared","profileId":p.id,"revision":files::revision(&snapshot(&root)),"processes":running,"expiresAt":now()+120});
            update_operation(&root, &op, &record)?;
            Ok(record)
        }
        "switch.commit" => {
            let _lock = files::lock(&root)?;
            file_mode(&root)?;
            let op = params["operationId"].as_str().context("缺少切换操作标识")?;
            let mut record = files::read(&operation(&root, op)?)?;
            if record["status"] == "succeeded" {
                return Ok(record);
            }
            if record["status"] == "committing" {
                ensure!(
                    files::revision(&files::read(&root.join("auth.json"))?)
                        == record["targetRevision"].as_str().unwrap_or(""),
                    "上次切换未完成，请重新检查当前账号"
                );
                record["status"] = json!("succeeded");
                update_operation(&root, op, &record)?;
                return Ok(record);
            }
            ensure!(
                record["status"] == "prepared"
                    && record["expiresAt"].as_i64().unwrap_or(0) >= now(),
                "切换检查已过期，请重新检查"
            );
            ensure!(
                record["revision"] == files::revision(&snapshot(&root)),
                "凭据已更新，请重新检查后切换"
            );
            let running: Vec<processes::Running> =
                serde_json::from_value(record["processes"].clone())?;
            ensure!(
                running.is_empty() || params["closeProcesses"] == true,
                "Codex 正在运行，尚未授权关闭"
            );
            processes::close(&root, &running)?;
            let profile = record["profileId"].as_str().context("目标档案缺失")?;
            let p = files::profile(&root, profile)?;
            let auth = read_authority(&root, &p)?;
            policy(&root, &auth)?;
            let query = layout(&root, profile, "query")?;
            {
                let mut rpc = Rpc::start(&loc.binary, &query, &loc.proxy, false)?;
                rpc.login_external(&auth)?;
                rpc.quota(&auth, &mut || Ok(auth.clone()))?;
            }
            ensure!(
                processes::list(&root)?.is_empty(),
                "Codex 已重新启动，账号未切换"
            );
            ensure!(
                record["revision"] == files::revision(&snapshot(&root)),
                "验证期间凭据已改变，请重新检查后切换"
            );
            let old = files::read(&root.join("auth.json")).ok();
            if let Some(ref old) = old {
                let ident = files::identity(old)?;
                let prior = files::profiles(&root)?
                    .into_iter()
                    .find(|p| p.identity.key == ident.key);
                let prior_id = prior
                    .as_ref()
                    .map(|p| p.id.clone())
                    .unwrap_or_else(files::new_id);
                files::save_profile(
                    &root,
                    &prior_id,
                    prior
                        .as_ref()
                        .map(|p| p.name.as_str())
                        .unwrap_or("之前的 ChatGPT"),
                    old,
                )?;
            }
            record["status"] = json!("committing");
            record["targetRevision"] = json!(files::revision(&auth));
            update_operation(&root, op, &record)?;
            ensure!(
                files::read(&root.join("auth.json"))
                    .ok()
                    .as_ref()
                    .map(files::revision)
                    == old.as_ref().map(files::revision),
                "外部凭据已变化，账号未切换"
            );
            files::atomic(&root.join("auth.json"), &auth)?;
            record["status"] = json!("succeeded");
            record["message"] = json!("账号已切换，请重新打开 Codex 并恢复会话");
            update_operation(&root, op, &record)?;
            Ok(record)
        }
        "profiles.remove" => {
            let _lock = files::lock(&root)?;
            recover(&root)?;
            let p = files::profile(&root, params["profileId"].as_str().context("请选择账号")?)?;
            ensure!(
                current(&root).as_deref() != Some(&p.identity.key),
                "不能移除日常使用中的账号，请先切换"
            );
            let path = files::profile_path(&root, &p.id)?;
            files::safe(&path)?;
            std::fs::remove_dir_all(path)?;
            let runtime = root.join(".aieyes/codex").join(&p.id);
            if runtime.exists() {
                files::safe(&runtime)?;
                std::fs::remove_dir_all(runtime)?;
            }
            Ok(json!({"removed":true}))
        }
        _ => anyhow::bail!("未知账号操作"),
    }
}

/// Holds the home lease through inference and persists rotated credentials even on failure.
pub fn with_credentials<T>(
    loc: &Location,
    profile_id: &str,
    body: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let root = loc.root()?;
    let _lock = files::lock(&root)?;
    file_mode(&root)?;
    recover(&root)?;
    let p = files::profile(&root, profile_id)?;
    let is_current = current(&root).as_deref() == Some(&p.identity.key);
    ensure!(
        !is_current || processes::list(&root)?.is_empty(),
        "日常 Codex 正在使用此账号，已跳过唤醒"
    );
    let auth = read_authority(&root, &p)?;
    let managed = layout(&root, profile_id, "managed")?;
    files::atomic(&managed.join("auth.json"), &auth)?;
    files::atomic(
        &root.join(".aieyes/state/refresh.json"),
        &json!({"profileId":profile_id,"current":is_current,"revision":files::revision(&auth)}),
    )?;
    let result = body(&managed);
    recover(&root)?;
    result
}
