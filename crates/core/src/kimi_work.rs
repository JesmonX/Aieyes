//! Kimi desktop's Work kernel uses a private Kimi Code home, not the CLI home.
//! Paths verified against Kimi 3.2.23's bundled runtime provider. Only the
//! storage pointer is read here; no desktop cookie or credential is decrypted.
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(crate) fn paths() -> Vec<(PathBuf, PathBuf)> {
    let home = crate::store::home();
    let app_data = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"))
    };
    let mut shares = vec![share_dir(&app_data.join("Kimi")), home.join(".kimi")];
    if let Some(path) = std::env::var_os("KIMI_SHARE_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        shares.push(path);
    }
    shares.sort();
    shares.dedup();
    shares
        .into_iter()
        .map(|share| {
            (
                share.join("daimon/runtime/kimi-code/home"),
                share.join("daimon/config.json"),
            )
        })
        .filter(|(home, config)| home.is_dir() || config.is_file())
        .collect()
}
fn share_dir(user_data: &Path) -> PathBuf {
    let value = std::fs::File::open(user_data.join("daimon-storage.json"))
        .ok()
        .and_then(|f| {
            let mut bytes = Vec::new();
            f.take(65537).read_to_end(&mut bytes).ok()?;
            (bytes.len() <= 65536)
                .then(|| serde_json::from_slice::<Value>(&bytes).ok())
                .flatten()
        });
    value
        .and_then(|v| v["shareDir"].as_str().map(PathBuf::from))
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| user_data.join("daimon-share"))
}

pub(crate) fn is_home(path: &Path) -> bool {
    path.ends_with("daimon/runtime/kimi-code/home")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_storage_relocation_and_torn_pointers_keep_read_only_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let pointer = temp.path().join("daimon-storage.json");
        assert_eq!(share_dir(temp.path()), temp.path().join("daimon-share"));
        let relocated = temp.path().join("账户数据");
        std::fs::write(
            &pointer,
            serde_json::json!({"shareDir":relocated}).to_string(),
        )
        .unwrap();
        assert_eq!(share_dir(temp.path()), relocated);
        std::fs::write(&pointer, "{half").unwrap();
        assert_eq!(share_dir(temp.path()), temp.path().join("daimon-share"));
        assert_eq!(std::fs::read_to_string(pointer).unwrap(), "{half");
        assert!(is_home(&relocated.join("daimon/runtime/kimi-code/home")));
    }
}
