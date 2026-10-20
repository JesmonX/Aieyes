//! One projection of account, machine and profile relationships for all clients.
use crate::{Engine, codex_auth};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

impl Engine {
    pub(crate) fn account_targets(&mut self, params: &Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let statuses = self.account_call("accounts.status.get", json!({}))?;
        let include_unavailable = params["includeUnavailable"] == true;
        let mut rows = vec![];
        for account in settings.accounts.iter().filter(|a| {
            !a.archived
                && params["accountKey"]
                    .as_str()
                    .is_none_or(|k| k == crate::accounts::key(a))
        }) {
            for source in settings.sources.iter().filter(|s| {
                s.provider == account.provider
                    && (include_unavailable || settings.source_enabled(s))
            }) {
                let available = settings.source_enabled(source);
                let cached: Option<String> = self
                    .store
                    .db
                    .query_row(
                        "SELECT value FROM kv WHERE key=?1",
                        [codex_auth::cli_probe_key(source, &settings, Some(account))],
                        |r| r.get(0),
                    )
                    .optional()?;
                let probe = cached
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .filter(|p| {
                        p["checkedAt"]
                            .as_i64()
                            .is_some_and(|t| (0..=120).contains(&(crate::models::now() - t)))
                    });
                let status = statuses.as_array().and_then(|rows| {
                    rows.iter().find(|r| {
                        r["accountKey"] == crate::accounts::key(account)
                            && r["sourceId"] == source.id
                    })
                });
                let refs: Vec<_> = account
                    .connections
                    .iter()
                    .filter(|c| c.source_id == source.id)
                    .filter_map(|c| c.profile_id.as_deref())
                    .collect();
                let profiles: Vec<Option<&str>> = if refs.is_empty() {
                    vec![None]
                } else {
                    refs.into_iter().map(Some).collect()
                };
                for profile in profiles {
                    let bound = if source.codex_home_id.is_some() {
                        profile.is_some()
                    } else {
                        source.account_id == account.id
                    };
                    let mut scoped = settings.clone();
                    scoped.accounts = vec![account.clone()];
                    let association_reason = codex_auth::validate_settings(&scoped)
                        .err()
                        .map(|e| e.to_string())
                        .or_else(|| {
                            profile
                                .filter(|p| {
                                    settings.accounts.iter().any(|a| {
                                        crate::accounts::key(a) != crate::accounts::key(account)
                                            && a.has_profile(p)
                                    })
                                })
                                .map(|_| "账户档案被多个账户引用，请重新关联".to_owned())
                        });
                    let draft_reason = association_reason.clone().or_else(|| {
                        if !bound {
                            Some("请先将此目录或账户档案关联到账户".to_owned())
                        } else {
                            crate::wakeups::source_for_draft(&settings, &source.id, profile)
                                .err()
                                .map(|error| error.to_string())
                        }
                    });
                    let reason = association_reason
                        .or_else(|| {
                            (!available).then(|| {
                                "目标机器或 Agent 已暂停或未选择，请在设置中恢复".to_owned()
                            })
                        })
                        .or_else(|| status.and_then(|r| r["error"].as_str().map(str::to_owned)))
                        .or_else(|| {
                            probe
                                .as_ref()
                                .filter(|p| p["cliAvailable"] == false)
                                .and_then(|p| p["cliError"].as_str().map(str::to_owned))
                        });
                    let credential = status.and_then(|r| {
                        if let Some((_, id)) = profile.and_then(|p| p.split_once(':')) {
                            r["profileCredentials"]
                                .as_object()
                                .map(|profiles| {
                                    profiles.get(id).and_then(Value::as_bool).unwrap_or(false)
                                })
                                .or_else(|| r["credential"].as_bool())
                        } else {
                            r["credential"].as_bool()
                        }
                    });
                    rows.push(json!({"accountKey":crate::accounts::key(account),"accountName":account.name,"provider":account.provider,
                        "sourceId":source.id,"sourceName":source.name,"path":source.path,"machineId":source.host_id.as_deref().unwrap_or("local"),
                        "machineName":source.host_id.as_ref().map(|id| settings.hosts.iter().find(|h| &h.id == id).map(|h| h.name.as_str()).unwrap_or_else(|| source.last_machine_name.as_deref().unwrap_or("已移除的机器"))).unwrap_or("本机"),
                        "profileId":profile,"current":status.map(|r| &r["current"]),"credential":credential,"checkedAt":status.map(|r| &r["checkedAt"]),
                        "bound":bound,"cliAvailable":probe.as_ref().map(|p|&p["cliAvailable"]),"canDraft":draft_reason.is_none(),"draftReason":draft_reason,"available":available,
                        "canWake":available && bound && credential != Some(false) && reason.is_none() && crate::providers::supports_wakeup(&account.provider),
                        "canSwitch":available && account.provider == "codex" && profile.is_some() && credential == Some(true) && reason.is_none() && status.is_some_and(|r| r["current"] != true),"reason":reason}));
                }
            }
        }
        Ok(json!(rows))
    }

    pub(crate) fn reconcile_codex(&mut self, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let source = settings
            .sources
            .iter()
            .find(|s| {
                params["sourceId"] == s.id && s.provider == "codex" && settings.source_enabled(s)
            })
            .context("请选择已启用的 Codex 来源")?
            .clone();
        let info = codex_auth::backend(&source, &settings, "status", &json!({}))?;
        ensure!(
            info["storageMode"] == "file",
            "此机器的认证存储不支持账户管理"
        );
        let accounts: Vec<_> = settings
            .accounts
            .iter()
            .filter(|a| {
                a.provider == "codex"
                    && !a.archived
                    && params["accountId"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .is_none_or(|id| a.id == id)
            })
            .cloned()
            .collect();
        let mut issues = vec![];
        for account in accounts {
            let Some(identity) = account.identity_key.as_deref() else {
                continue;
            };
            let stored = info["profiles"]
                .as_array()
                .and_then(|ps| {
                    ps.iter()
                        .find(|p| p["identityKey"] == identity && p["credential"] == true)
                })
                .and_then(|p| p["id"].as_str());
            let current = info["currentIdentity"] == identity && info["currentCredential"] == true;
            if !current && stored.is_none() {
                continue;
            }
            let result = (|| -> Result<()> {
                let _critical = crate::operations::critical()?;
                self.codex_auth_call("codexAuth.enable", json!({"sourceId":source.id}))?;
                if let Some(profile) = stored {
                    self.codex_auth_call(
                        "codexAuth.profiles.bind",
                        json!({"sourceId":source.id,"profileId":profile,"accountId":account.id}),
                    )?;
                } else {
                    // adopt verifies the current credentials and never switches auth.json.
                    self.codex_auth_call(
                        "codexAuth.adopt",
                        json!({"sourceId":source.id,"accountId":account.id}),
                    )?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                issues.push(json!({"accountKey":crate::accounts::key(&account),"sourceId":source.id,"message":error.to_string()}));
            }
        }
        for account in settings.accounts.iter().filter(|a| {
            a.provider == "codex"
                && !a.archived
                && params["accountId"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .is_none_or(|id| a.id == id)
        }) {
            crate::operations::check()?;
            match self.account_call("accounts.status.refresh", json!({"accountKey":crate::accounts::key(account),"sourceId":source.id})) {
                Ok(row) if row["error"].is_string() => issues.push(json!({"accountKey":crate::accounts::key(account),"sourceId":source.id,"message":row["error"]})),
                Err(error) => issues.push(json!({"accountKey":crate::accounts::key(account),"sourceId":source.id,"message":error.to_string()})),
                _ => {}
            }
        }
        Ok(
            json!({"settings":self.store.settings()?,"targets":self.account_targets(&json!({}))?,"issues":issues}),
        )
    }
}
