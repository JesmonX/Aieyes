//! One projection of account, machine and profile relationships for all clients.
use crate::{Engine, codex_auth};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

impl Engine {
    pub(crate) fn account_targets(&mut self, params: &Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let statuses = self.account_call("accounts.status.get", json!({}))?;
        let mut rows = vec![];
        for account in settings.accounts.iter().filter(|a| !a.archived && params["accountKey"].as_str().is_none_or(|k| k == crate::accounts::key(a))) {
            let Some(agent) = settings.agents.iter().find(|a| a.provider == account.provider && a.enabled) else { continue };
            for source in settings.sources.iter().filter(|s| s.provider == account.provider && s.enabled
                && agent.machine_ids.iter().any(|id| id == s.host_id.as_deref().unwrap_or("local"))
                && s.host_id.as_ref().is_none_or(|id| settings.hosts.iter().any(|h| &h.id == id && h.enabled))) {
                let status = statuses.as_array().and_then(|rows| rows.iter().find(|r| r["accountKey"] == crate::accounts::key(account) && r["sourceId"] == source.id));
                let refs: Vec<_> = account.connections.iter().filter(|c| c.source_id == source.id).filter_map(|c| c.profile_id.as_deref()).collect();
                let profiles: Vec<Option<&str>> = if refs.is_empty() { vec![None] } else { refs.into_iter().map(Some).collect() };
                for profile in profiles {
                    let bound = if source.codex_home_id.is_some() { profile.is_some() } else { source.account_id == account.id };
                    let reason = settings.configuration_issues.first().cloned().or_else(|| status.and_then(|r| r["error"].as_str().map(str::to_owned)));
                    rows.push(json!({"accountKey":crate::accounts::key(account),"accountName":account.name,"provider":account.provider,
                        "sourceId":source.id,"sourceName":source.name,"path":source.path,"machineId":source.host_id.as_deref().unwrap_or("local"),
                        "machineName":source.host_id.as_ref().and_then(|id| settings.hosts.iter().find(|h| &h.id == id)).map(|h| h.name.as_str()).unwrap_or("本机"),
                        "profileId":profile,"current":status.map(|r| &r["current"]),"credential":status.map(|r| &r["credential"]),"checkedAt":status.map(|r| &r["checkedAt"]),
                        "canWake":bound && reason.is_none() && crate::providers::supports_wakeup(&account.provider),
                        "canSwitch":account.provider == "codex" && profile.is_some() && reason.is_none() && status.is_some_and(|r| r["credential"] == true && r["current"] != true),"reason":reason}));
                }
            }
        }
        Ok(json!(rows))
    }

    pub(crate) fn reconcile_codex(&mut self, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let source = settings.sources.iter().find(|s| params["sourceId"] == s.id && s.provider == "codex" && s.enabled).context("请选择已启用的 Codex 来源")?.clone();
        let info = codex_auth::backend(&source, &settings, "status", &json!({}))?;
        ensure!(info["storageMode"] == "file", "此机器的认证存储不支持账号管理");
        let accounts: Vec<_> = settings.accounts.iter().filter(|a| a.provider == "codex" && !a.archived && params["accountId"].as_str().filter(|id| !id.is_empty()).is_none_or(|id| a.id == id)).cloned().collect();
        let mut issues = vec![];
        for account in accounts {
            let Some(identity) = account.identity_key.as_deref() else { continue };
            let stored = info["profiles"].as_array().and_then(|ps| ps.iter().find(|p| p["identityKey"] == identity && p["credential"] == true)).and_then(|p| p["id"].as_str());
            let current = info["currentIdentity"] == identity && info["currentCredential"] == true;
            if !current && stored.is_none() { continue; }
            let result = (|| -> Result<()> {
                self.codex_auth_call("codexAuth.enable", json!({"sourceId":source.id}))?;
                if let Some(profile) = stored {
                    self.codex_auth_call("codexAuth.profiles.bind", json!({"sourceId":source.id,"profileId":profile,"accountId":account.id}))?;
                } else {
                    // adopt verifies the current credentials and never switches auth.json.
                    self.codex_auth_call("codexAuth.adopt", json!({"sourceId":source.id,"accountId":account.id}))?;
                }
                Ok(())
            })();
            if let Err(error) = result { issues.push(json!({"accountKey":crate::accounts::key(&account),"sourceId":source.id,"message":error.to_string()})); }
        }
        for account in settings.accounts.iter().filter(|a| a.provider == "codex" && !a.archived) {
            let _ = self.account_call("accounts.status.refresh", json!({"accountKey":crate::accounts::key(account),"sourceId":source.id}));
        }
        Ok(json!({"settings":self.store.settings()?,"targets":self.account_targets(&json!({}))?,"issues":issues}))
    }
}
