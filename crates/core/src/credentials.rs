//! SSH secrets live in the operating system's credential store, never in settings.
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, sync::OnceLock};

static HELPER: OnceLock<PathBuf> = OnceLock::new();
pub fn set_helper(path: PathBuf) {
    let _ = HELPER.set(path);
}
pub fn helper() -> Result<PathBuf> {
    HELPER
        .get()
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| std::env::current_exe().context("未找到 SSH 登录辅助程序"))
}
#[cfg(not(test))]
type SecretEntry = keyring::Entry;
#[cfg(test)]
type SecretEntry = keyring_core::Entry;
fn entry(reference: &str) -> Result<SecretEntry> {
    #[cfg(test)]
    {
        static MOCK: std::sync::Once = std::sync::Once::new();
        MOCK.call_once(|| {
            keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap())
        });
    }
    ensure!(
        !reference.is_empty()
            && reference
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "密码引用无效"
    );
    SecretEntry::new("app.aieyes.ssh", reference).context("系统凭据库不可用")
}

pub fn save(password: &str) -> Result<String> {
    ensure!(
        !password.is_empty() && !password.contains(['\n', '\r', '\0']),
        "请输入有效的服务器密码"
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let reference = format!("ssh-{}-{stamp}", std::process::id());
    entry(&reference)?
        .set_password(password)
        .context("无法保存服务器密码，请检查系统凭据库是否已解锁")?;
    Ok(reference)
}
pub fn read(reference: &str) -> Result<String> {
    entry(reference)?
        .get_password()
        .context("无法读取服务器密码，请重新输入或解锁系统凭据库")
}
pub fn delete(reference: &str) -> Result<()> {
    entry(reference)?
        .delete_credential()
        .context("无法清理服务器密码")
}
/// Runs before opening SQLite. OpenSSH passes only its prompt; the environment
/// contains an opaque keyring reference, not the password.
pub fn askpass() -> Result<bool> {
    let Ok(reference) = std::env::var("AIEYES_SSH_ASKPASS_REF") else {
        return Ok(false);
    };
    let prompt = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    ensure!(
        prompt.contains("password") && !prompt.contains("passphrase"),
        "此登录提示需要手动处理"
    );
    use std::io::Write;
    writeln!(std::io::stdout().lock(), "{}", read(&reference)?)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passwords_are_opaque_and_ssh_never_receives_a_secret_in_arguments_or_environment() {
        let password = " fixture $secret `value` ";
        let reference = save(password).unwrap();
        assert_eq!(read(&reference).unwrap(), password);
        let replacement = save("new-password").unwrap();
        assert_ne!(reference, replacement);
        assert_eq!(
            read(&reference).unwrap(),
            password,
            "draft replacement must not overwrite the committed password"
        );
        let host = crate::models::Host {
            target: "fixture.invalid".into(),
            username: "fixture-user".into(),
            auth_mode: "password".into(),
            password_ref: reference.clone(),
            ..Default::default()
        };
        let command = crate::ssh::command(&host, "true").unwrap();
        let args = command
            .get_args()
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            args.contains("BatchMode=no") && args.contains("PreferredAuthentications=password")
        );
        assert!(!args.contains(password));
        assert!(
            command
                .get_envs()
                .all(|(_, v)| v.is_none_or(|v| !v.to_string_lossy().contains(password)))
        );
        delete(&reference).unwrap();
        assert!(read(&reference).is_err());
        delete(&replacement).unwrap();
        assert!(save("bad\npassword").is_err());
        assert!(read("../invalid").is_err());
    }
}
