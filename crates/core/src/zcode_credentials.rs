//! ZCode v2's shared credential codec, read only. No client process or refresh.
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const INVALID: &str = "ZCode 登录凭据无法解密，请检查原生凭据与 ZCODE_CREDENTIAL_SECRET";

fn allowed(key: &str) -> bool {
    matches!(
        key,
        "oauth:active_provider"
            | "zcodejwttoken"
            | "oauth:zai:user_info"
            | "oauth:bigmodel:user_info"
    ) || key.starts_with("account-provider:")
        && (key.ends_with(":identity")
            || key.ends_with(":auth-source")
            || key.starts_with("account-provider:coding-plan:") && key.ends_with(":api-key"))
}

fn username() -> String {
    #[cfg(unix)]
    {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0_u8; 16384];
        // getpwuid_r owns no returned allocation; pw_name lives in buffer.
        let ok = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if ok == 0 && !result.is_null() {
            let name = unsafe { std::ffi::CStr::from_ptr((*result).pw_name) };
            return name.to_string_lossy().into_owned();
        }
    }
    #[cfg(windows)]
    if let Ok(name) = std::env::var("USERNAME") {
        return name;
    }
    "unknown".into()
}

fn native_secret() -> String {
    if let Ok(secret) = std::env::var("ZCODE_CREDENTIAL_SECRET")
        && !secret.is_empty()
    {
        return secret;
    }
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        std::env::consts::OS
    };
    format!(
        "zcode-credential-fallback:{platform}:{}:{}",
        crate::store::home().display(),
        username()
    )
}

fn decrypt(value: &str, secret: &str) -> Result<String> {
    let Some(ciphertext) = value.strip_prefix("enc:v1:") else {
        ensure!(!value.starts_with("enc:"), INVALID);
        return Ok(value.into());
    };
    let parts: Vec<_> = ciphertext.split('.').collect();
    ensure!(parts.len() == 3, INVALID);
    let decode = |s: &str| {
        URL_SAFE_NO_PAD
            .decode(s.trim_end_matches('='))
            .map_err(|_| anyhow::anyhow!(INVALID))
    };
    let nonce = decode(parts[0])?;
    let tag = decode(parts[1])?;
    let mut bytes = decode(parts[2])?;
    ensure!(
        nonce.len() == 12 && tag.len() == 16 && bytes.len() <= 32768,
        INVALID
    );
    bytes.extend(tag);
    let key = UnboundKey::new(&AES_256_GCM, &Sha256::digest(secret.as_bytes()))
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    let nonce = Nonce::try_assume_unique_for_key(&nonce).map_err(|_| anyhow::anyhow!(INVALID))?;
    let plain = LessSafeKey::new(key)
        .open_in_place(nonce, Aad::empty(), &mut bytes)
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    String::from_utf8(plain.to_vec()).map_err(|_| anyhow::anyhow!(INVALID))
}

fn project_with_secret(value: &Value, secret: &str) -> Result<Value> {
    let map = value.as_object().context(INVALID)?;
    let mut out = Map::new();
    for (key, value) in map.iter().filter(|(key, _)| allowed(key)) {
        let value = value.as_str().context(INVALID)?;
        out.insert(key.clone(), Value::String(decrypt(value, secret)?));
    }
    Ok(Value::Object(out))
}

pub(crate) fn project(value: &Value, local: bool) -> Result<Value> {
    // SSH decrypts on the owning machine with its own home/user/environment.
    // Never try a local fallback key for ciphertext from another machine.
    if !local {
        ensure!(
            value.as_object().is_some_and(|map| map
                .iter()
                .filter(|(k, _)| allowed(k))
                .all(|(_, v)| v.as_str().is_some_and(|s| !s.starts_with("enc:")))),
            INVALID
        );
    }
    project_with_secret(
        value,
        &if local {
            native_secret()
        } else {
            String::new()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_native_aes_gcm_and_never_projects_refresh_tokens() {
        let key = UnboundKey::new(&AES_256_GCM, &Sha256::digest(b"test-secret")).unwrap();
        let nonce = [5_u8; 12];
        let mut data = b"zai".to_vec();
        let tag = LessSafeKey::new(key)
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut data,
            )
            .unwrap();
        let encrypted = format!(
            "enc:v1:{}.{}.{}",
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(tag.as_ref()),
            URL_SAFE_NO_PAD.encode(data)
        );
        let value =
            json!({"oauth:active_provider":encrypted,"oauth:zai:refresh_token":"private-refresh"});
        // Independent vector from the installed Node cipher (SHA-256 + AES-GCM).
        assert_eq!(
            value["oauth:active_provider"],
            "enc:v1:BQUFBQUFBQUFBQUF.N6rUk4dZmqwly8-sZzrbtA.ffbf"
        );
        let projected = project_with_secret(&value, "test-secret").unwrap();
        assert_eq!(projected, json!({"oauth:active_provider":"zai"}));
        assert!(project_with_secret(&value, "wrong-secret").is_err());
        assert!(project(&value, false).is_err());
        assert!(decrypt("enc:v2:future", "test-secret").is_err());
    }
}
