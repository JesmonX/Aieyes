//! Private files and cross-process locks. Never decide freshness by mtime.
use crate::models::{hash, now};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub fn new_id() -> String {
    hash(&format!(
        "{}:{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))[..32]
        .into()
}
pub fn id(value: &str) -> Result<&str> {
    ensure!(
        !value.is_empty()
            && value.len() <= 80
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "管理标识无效"
    );
    Ok(value)
}
pub fn safe(path: &Path) -> Result<()> {
    let mut p = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, std::path::Component::ParentDir),
            "管理路径不能包含上级目录"
        );
        p.push(part);
        if let Ok(m) = fs::symlink_metadata(&p) {
            ensure!(!m.file_type().is_symlink(), "管理路径不能包含符号链接");
        }
    }
    Ok(())
}
pub fn private_dir(path: &Path) -> Result<()> {
    safe(path)?;
    if !path.exists()
        && let Some(parent) = path.parent()
        && parent.file_name().is_some_and(|n| {
            n == ".aieyes" || parent.components().any(|c| c.as_os_str() == ".aieyes")
        })
    {
        private_dir(parent)?;
    }
    fs::create_dir_all(path).context("无法创建账号管理目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            fs::metadata(path)?.uid() == unsafe { libc::geteuid() },
            "管理目录不属于当前用户"
        );
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    restrict_acl(path, true)?;
    Ok(())
}
#[cfg(windows)]
fn restrict_acl(path: &Path, directory: bool) -> Result<()> {
    let who = std::env::var("USERNAME").context("无法读取当前 Windows 用户")?;
    // Reset explicit rules, then discard inherited rules before writing any bytes.
    let mut reset = std::process::Command::new("icacls");
    reset.arg(path).arg("/reset");
    crate::process::run(reset, vec![], std::time::Duration::from_secs(10))?;
    let mut restrict = std::process::Command::new("icacls");
    restrict.arg(path).args([
        "/inheritance:r",
        "/grant:r",
        &format!("{who}:{}F", if directory { "(OI)(CI)" } else { "" }),
    ]);
    crate::process::run(restrict, vec![], std::time::Duration::from_secs(10))?;
    Ok(())
}
pub fn read(path: &Path) -> Result<Value> {
    safe(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .context("凭据或管理记录不存在")?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "管理记录过大");
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("凭据或管理记录格式无效"))
}
pub fn revision(value: &Value) -> String {
    hash(&value.to_string())
}
pub fn atomic(path: &Path, value: &Value) -> Result<()> {
    safe(path)?;
    // Do not chmod CODEX_HOME itself: only the Aieyes-owned staging directory.
    let parent = path.parent().context("管理路径无效")?;
    let temp = parent.join(format!(".aieyes-{}.tmp", new_id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut f = options.open(&temp)?;
        #[cfg(windows)]
        restrict_acl(&temp, false)?;
        f.write_all(serde_json::to_string(value)?.as_bytes())?;
        f.sync_all()?;
        #[cfg(not(windows))]
        fs::rename(&temp, path)?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
            }
            let from: Vec<_> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            ensure!(
                unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } != 0,
                "无法原子替换凭据文件"
            );
        }
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    result.map_err(|_| anyhow::anyhow!("无法安全保存凭据或管理记录"))
}
pub fn lock(root: &Path) -> Result<crate::file_lock::FileLock> {
    let state = root.join(".aieyes/state");
    private_dir(&state)?;
    let path = state.join("coordinator.lock");
    safe(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let f = options.open(path)?;
    crate::file_lock::FileLock::try_exclusive(f)
        .map_err(|_| anyhow::anyhow!("此 Codex 目录有正在执行的账号操作，请稍后重试"))
}
fn claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or(Value::Null)
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub key: String,
    pub workspace: String,
    pub email: String,
    pub plan: String,
}
pub fn identity(auth: &Value) -> Result<Identity> {
    ensure!(
        auth["auth_mode"] != "apikey",
        "需要 ChatGPT 订阅登录，不能使用 API Key"
    );
    let tokens = &auth["tokens"];
    let access = tokens["access_token"]
        .as_str()
        .context("缺少 ChatGPT 访问凭据")?;
    let a = claims(access);
    let i = claims(tokens["id_token"].as_str().unwrap_or(""));
    let meta = &a["https://api.openai.com/auth"];
    let imeta = &i["https://api.openai.com/auth"];
    let workspace = tokens["account_id"]
        .as_str()
        .or(meta["chatgpt_account_id"].as_str())
        .or(imeta["chatgpt_account_id"].as_str())
        .filter(|s| !s.is_empty())
        .context("无法确认 ChatGPT 工作区")?;
    let user = meta["chatgpt_user_id"]
        .as_str()
        .or(imeta["chatgpt_user_id"].as_str())
        .or(i["sub"].as_str())
        .filter(|s| !s.is_empty())
        .context("无法确认 ChatGPT 用户身份，请重新登录")?;
    Ok(Identity {
        key: hash(&format!("{user}\0{workspace}")),
        workspace: workspace.into(),
        email: i["email"]
            .as_str()
            .or(a["https://api.openai.com/profile"]["email"].as_str())
            .unwrap_or("")
            .into(),
        plan: meta["chatgpt_plan_type"]
            .as_str()
            .or(imeta["chatgpt_plan_type"].as_str())
            .unwrap_or("unknown")
            .into(),
    })
}
pub fn external(auth: &Value) -> Result<Value> {
    let ident = identity(auth)?;
    Ok(
        json!({"type":"chatgptAuthTokens", "accessToken":auth["tokens"]["access_token"], "chatgptAccountId":ident.workspace, "chatgptPlanType":ident.plan}),
    )
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub identity: Identity,
    pub updated_at: i64,
}
pub fn profile_path(root: &Path, profile: &str) -> Result<PathBuf> {
    Ok(root.join(".aieyes/accounts").join(id(profile)?))
}
pub fn profile(root: &Path, profile: &str) -> Result<Profile> {
    serde_json::from_value(read(&profile_path(root, profile)?.join("metadata.json"))?)
        .context("账号档案格式无效")
}
pub fn save_profile(root: &Path, profile: &str, name: &str, auth: &Value) -> Result<Profile> {
    let path = profile_path(root, profile)?;
    private_dir(&path)?;
    let ident = identity(auth)?;
    if path.join("metadata.json").exists() {
        ensure!(
            self::profile(root, profile)?.identity.key == ident.key,
            "账号身份发生变化，原档案已保留"
        );
    }
    let p = Profile {
        id: profile.into(),
        name: if !name.trim().is_empty() {
            name.trim().into()
        } else if !ident.email.trim().is_empty() {
            ident.email.trim().into()
        } else {
            "Codex 账户".into()
        },
        identity: ident,
        updated_at: now(),
    };
    atomic(&path.join("auth.json"), auth)?;
    atomic(&path.join("metadata.json"), &serde_json::to_value(&p)?)?;
    Ok(p)
}
pub fn profiles(root: &Path) -> Result<Vec<Profile>> {
    let path = root.join(".aieyes/accounts");
    if !path.exists() {
        return Ok(vec![]);
    }
    safe(&path)?;
    let mut rows = Vec::new();
    for e in fs::read_dir(path)? {
        let e = e?;
        if e.file_type()?.is_dir() && e.path().join("metadata.json").is_file() {
            rows.push(profile(root, &e.file_name().to_string_lossy())?);
        }
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rows)
}
