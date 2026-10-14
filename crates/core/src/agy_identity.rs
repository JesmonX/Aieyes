//! Public identity observations; the helper keeps local/SSH credentials on their machine.
use crate::{accounts, models::*, network, process, ssh};
use serde::Deserialize;
use std::time::Duration;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Observation {
    pub identity: Option<AccountIdentity>,
    pub metadata_error: Option<String>,
}

pub(crate) fn read(source: &Source, settings: &Settings) -> Observation {
    read_inner(source, settings, false)
}
pub(crate) fn read_key(source: &Source, settings: &Settings) -> Option<String> {
    read_inner(source, settings, true).identity.map(|id| id.key)
}
fn read_inner(source: &Source, settings: &Settings, identity_only: bool) -> Observation {
    let result = (|| -> anyhow::Result<Observation> {
        let script = include_str!("../../../scripts/agy_identity.py");
        let flag = if identity_only {
            " --identity-only"
        } else {
            ""
        };
        let account = settings
            .accounts
            .iter()
            .find(|a| a.provider == source.provider && a.id == source.account_id);
        let pre = accounts::pre_command(settings, account, source);
        let mut cmd = if let Some(id) = &source.host_id {
            let mut host = settings
                .hosts
                .iter()
                .find(|h| &h.id == id && h.enabled)
                .ok_or_else(|| anyhow::anyhow!("设备已暂停"))?
                .clone();
            host.pre_command = pre;
            ssh::account_command(&host, &format!("exec python3 -{flag}"))?
        } else {
            let args = if identity_only {
                vec!["-", "--identity-only"]
            } else {
                vec!["-"]
            };
            let mut cmd = process::cli_command_with_pre_command(
                if cfg!(windows) { "python" } else { "python3" },
                &args,
                &pre,
                ssh::DEFAULT_SHELL,
            );
            network::apply_env(&mut cmd, source.proxy.as_ref().unwrap_or(&settings.proxy));
            cmd
        };
        cmd.current_dir(std::env::temp_dir());
        let raw = process::run(cmd, script.as_bytes().to_vec(), Duration::from_secs(25))?;
        // Deserialize a whitelist; neither raw JSON nor subprocess errors reach callers.
        let observation: Observation = serde_json::from_slice(&raw)?;
        if let Some(id) = &observation.identity {
            anyhow::ensure!(
                id.key.starts_with("google:") && id.key.len() == 71 && id.email.contains('@'),
                "身份格式无效"
            );
        }
        Ok(observation)
    })();
    result.unwrap_or_else(|_| Observation {
        metadata_error: Some("身份信息暂不可用，请检查 Python 3、curl 与登录凭据库".into()),
        ..Default::default()
    })
}

pub(crate) fn matches(account: Option<&Account>, identity: &AccountIdentity) -> bool {
    account
        .and_then(|a| a.identity_key.as_deref())
        .is_none_or(|key| key == identity.key)
}
