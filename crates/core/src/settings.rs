//! Small configuration updates, merged under a write transaction.
use crate::{Engine, models::*, store::Store};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

/// Pure local configuration work must not wait behind remote requests.
pub fn is_configuration_method(method: &str) -> bool {
    matches!(
        method,
        "settings.get"
            | "settings.patch"
            | "settings.save"
            | "agents.set"
            | "sources.configure"
            | "sources.remove"
            | "accounts.connect"
            | "accounts.create"
            | "accounts.status.get"
            | "accounts.deletion.preview"
            | "accounts.cleanup.list"
    )
}

// Preserve concurrent edits to unrelated fields and rows. A conflicting field
// fails before any configuration is written; clients retain their form input.
fn merge(base: &Value, next: &Value, current: &Value, path: &str) -> Result<Value> {
    if base == next {
        return Ok(current.clone());
    }
    if current == base || current == next {
        return Ok(next.clone());
    }
    if let (Some(b), Some(n), Some(c)) = (base.as_object(), next.as_object(), current.as_object()) {
        let mut result = c.clone();
        for key in b
            .keys()
            .chain(n.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let value = merge(
                b.get(key).unwrap_or(&Value::Null),
                n.get(key).unwrap_or(&Value::Null),
                c.get(key).unwrap_or(&Value::Null),
                &format!("{path}.{key}"),
            )?;
            if value.is_null() && !n.contains_key(key) {
                result.remove(key);
            } else {
                result.insert(key.clone(), value);
            }
        }
        return Ok(Value::Object(result));
    }
    if let (Some(b), Some(n), Some(c)) = (base.as_array(), next.as_array(), current.as_array()) {
        let identity = |v: &Value| -> Option<String> {
            v["id"]
                .as_str()
                .map(|id| format!("{}:{id}", v["provider"].as_str().unwrap_or("")))
                .or_else(|| v["provider"].as_str().map(str::to_owned))
                .or_else(|| v["machineId"].as_str().map(str::to_owned))
        };
        if b.iter().chain(n).chain(c).all(|v| identity(v).is_some()) {
            let mut result = c.clone();
            for id in b
                .iter()
                .chain(n)
                .filter_map(identity)
                .collect::<std::collections::BTreeSet<_>>()
            {
                let old = b
                    .iter()
                    .find(|v| identity(v).as_deref() == Some(&id))
                    .unwrap_or(&Value::Null);
                let new = n
                    .iter()
                    .find(|v| identity(v).as_deref() == Some(&id))
                    .unwrap_or(&Value::Null);
                let index = result
                    .iter()
                    .position(|v| identity(v).as_deref() == Some(&id));
                let now = index.map(|i| &result[i]).unwrap_or(&Value::Null);
                let value = merge(old, new, now, &format!("{path}[{id}]"))?;
                if let Some(i) = index {
                    if value.is_null() {
                        result.remove(i);
                    } else {
                        result[i] = value;
                    }
                } else if !value.is_null() {
                    result.push(value);
                }
            }
            return Ok(Value::Array(result));
        }
    }
    anyhow::bail!("此设置已在其他窗口修改：{path}。输入已保留，请重新读取后重试")
}

impl Store {
    pub fn patch_settings(&self, base: &Value, next: &Value, auth: bool) -> Result<Settings> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let normalize = |v: &Value| -> Result<Value> {
            let mut s: Settings = serde_json::from_value(v.clone())?;
            crate::providers::normalize_settings(&mut s);
            Ok(serde_json::to_value(s)?)
        };
        let current = serde_json::to_value(self.settings()?)?;
        let mut result: Settings = serde_json::from_value(merge(
            &normalize(base)?,
            &normalize(next)?,
            &current,
            "设置",
        )?)?;
        result.deleted_accounts = serde_json::from_value(current["deletedAccounts"].clone())?;
        result.account_aliases = serde_json::from_value(current["accountAliases"].clone())?;
        if auth {
            self.save_auth_settings(&result)?;
        } else {
            self.save_settings(&result)?;
        }
        let saved = self.settings()?;
        tx.commit()?;
        // Credential deletion is external to SQLite and only follows a successful commit.
        let old: Settings = serde_json::from_value(current)?;
        for host in old.hosts {
            if !host.password_ref.is_empty()
                && !saved
                    .hosts
                    .iter()
                    .any(|h| h.password_ref == host.password_ref)
            {
                let _ = crate::credentials::delete(&host.password_ref);
            }
        }
        Ok(saved)
    }
}

impl Engine {
    /// Directory changes are local transactions; never delete files or credentials.
    pub(crate) fn configure_source(&mut self, params: Value, remove: bool) -> Result<Value> {
        let _tasks = crate::accounts::lock_tasks(&self.store)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.store.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let base = self.store.settings()?;
        let mut next = base.clone();
        let mut source: Source = if remove {
            base.sources
                .iter()
                .find(|s| Some(s.id.as_str()) == params["sourceId"].as_str())
                .context("目录配置已移除")?
                .clone()
        } else {
            serde_json::from_value(params["source"].clone())?
        };
        let old = base.sources.iter().find(|s| s.id == source.id);
        if let Some(expected) = params.get("baseSource") {
            ensure!(
                old.map(serde_json::to_value).transpose()?
                    == Some(serde_json::to_value(serde_json::from_value::<Source>(
                        expected.clone()
                    )?)?),
                "目录配置已在其他窗口修改，请重新打开后重试"
            );
        }
        if let Some(old) = old {
            ensure!(
                old.provider == source.provider && old.host_id == source.host_id,
                "请在对应机器下添加目录"
            );
            ensure!(
                old.codex_home_id == source.codex_home_id && old.account_id == source.account_id,
                "账户关联请通过账户设置修改"
            );
        } else {
            ensure!(
                source.codex_home_id.is_none(),
                "请在账户中读取新目录的登录信息"
            );
        }
        let moved = old.is_some_and(|s| s.path != source.path);
        if remove || moved {
            let tasks: Vec<_> = self
                .store
                .wakeups()?
                .into_iter()
                .filter(|r| {
                    r.task.source_id == source.id
                        || r.deployment
                            .as_ref()
                            .is_some_and(|d| d.source.id == source.id)
                })
                .map(|r| r.task.name)
                .collect();
            ensure!(
                tasks.is_empty(),
                "目录仍被唤醒任务引用，请先解除引用：{}",
                tasks.join("、")
            );
        }
        let detach = remove || (moved && old.is_some_and(|s| s.codex_home_id.is_some()));
        if detach {
            let home = old.and_then(|s| s.codex_home_id.as_ref());
            for account in &mut next.accounts {
                account.connections.retain(|c| c.source_id != source.id);
                if account.quota_source_id.as_ref() == Some(&source.id) {
                    account.quota_source_id = None;
                }
                if home.is_some_and(|h| {
                    account
                        .quota_profile_id
                        .as_ref()
                        .is_some_and(|p| p.starts_with(&format!("{h}:")))
                }) {
                    account.quota_profile_id = None;
                }
            }
            next.sources.retain(|s| s.id != source.id);
            self.store
                .db
                .execute("DELETE FROM kv WHERE key LIKE 'account-status:%'", [])?;
            if !remove {
                source.id = format!("agent-{}", crate::codex_auth::files::new_id());
                source.codex_home_id = None;
                source.account_id.clear();
            }
        }
        if !remove {
            ensure!(
                !next.sources.iter().any(|s| s.id != source.id
                    && s.provider == source.provider
                    && s.host_id == source.host_id
                    && s.path == source.path),
                "此机器已配置该目录"
            );
            let agent = next
                .agents
                .iter()
                .find(|a| a.provider == source.provider)
                .context("不支持的 Agent")?;
            source.enabled = agent.enabled
                && agent
                    .machine_ids
                    .iter()
                    .any(|id| id == source.host_id.as_deref().unwrap_or("local"));
            if let Some(i) = next.sources.iter().position(|s| s.id == source.id) {
                next.sources[i] = source.clone();
            } else {
                next.sources.push(source.clone());
            }
        } else if !next
            .sources
            .iter()
            .any(|s| s.provider == source.provider && s.host_id == source.host_id)
            && let Some(agent) = next
                .agents
                .iter_mut()
                .find(|a| a.provider == source.provider)
        {
            agent
                .machine_ids
                .retain(|id| id != source.host_id.as_deref().unwrap_or("local"));
        }
        self.store.save_auth_settings(&next)?;
        let saved = self.store.settings()?;
        tx.commit()?;
        Ok(serde_json::to_value(saved)?)
    }

    pub(crate) fn configure_agent(&mut self, params: Value) -> Result<Value> {
        let base = self.store.settings()?;
        let mut next = base.clone();
        let provider = params["provider"].as_str().context("请选择 Agent")?;
        let agent = next
            .agents
            .iter_mut()
            .find(|a| a.provider == provider)
            .context("不支持的 Agent")?;
        if let Some(enabled) = params["enabled"].as_bool() {
            agent.enabled = enabled;
        }
        if let Some(ids) = params.get("machineIds") {
            agent.machine_ids = serde_json::from_value(ids.clone())?;
        }
        for id in &agent.machine_ids {
            ensure!(
                id == "local" || next.hosts.iter().any(|h| &h.id == id),
                "机器已删除，请重新选择"
            );
            ensure!(
                provider != "deepseek" || id == "local",
                "DeepSeek 余额使用本机 API Key 查询"
            );
        }
        agent.machine_ids.sort();
        agent.machine_ids.dedup();
        for machine in &agent.machine_ids {
            if !next.sources.iter().any(|s| {
                s.provider == provider && s.host_id.as_deref().unwrap_or("local") == machine
            }) {
                next.sources.push(Source {
                    id: format!("agent-{}", crate::codex_auth::files::new_id()),
                    name: format!(
                        "{} · {}",
                        crate::providers::display_name(provider),
                        if machine == "local" {
                            "本机"
                        } else {
                            &next.hosts.iter().find(|h| &h.id == machine).unwrap().name
                        }
                    ),
                    provider: provider.into(),
                    path: match provider {
                        "codex" => "~/.codex",
                        "claude" => "~/.claude",
                        "antigravity" => "~/.gemini/antigravity-cli",
                        "custom" => "~/.aieyes",
                        _ => "",
                    }
                    .into(),
                    host_id: (machine != "local").then(|| machine.clone()),
                    ..Default::default()
                });
            }
        }
        for source in next.sources.iter_mut().filter(|s| s.provider == provider) {
            source.enabled = agent.enabled
                && agent
                    .machine_ids
                    .iter()
                    .any(|id| id == source.host_id.as_deref().unwrap_or("local"));
        }
        Ok(serde_json::to_value(self.store.patch_settings(
            &serde_json::to_value(base)?,
            &serde_json::to_value(next)?,
            false,
        )?)?)
    }

    pub(crate) fn connect_account(&mut self, params: Value) -> Result<Value> {
        let base = self.store.settings()?;
        let mut next = base.clone();
        let provider = params["provider"].as_str().context("请选择 Agent")?;
        ensure!(
            next.agents
                .iter()
                .any(|a| a.provider == provider && a.enabled),
            "请先启用此 Agent"
        );
        let account_id = params["accountId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(crate::codex_auth::files::new_id);
        let source_ids: Vec<String> = serde_json::from_value(params["sourceIds"].clone())?;
        ensure!(!source_ids.is_empty(), "请选择已启用的机器连接");
        if !next
            .accounts
            .iter()
            .any(|a| a.id == account_id && a.provider == provider)
        {
            next.accounts.push(Account {
                id: account_id.clone(),
                provider: provider.into(),
                name: params["name"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| crate::accounts::default_name(provider)),
                ..Default::default()
            });
        }
        for id in source_ids {
            let source = next
                .sources
                .iter_mut()
                .find(|s| s.id == id && s.provider == provider && s.enabled)
                .context("请选择已启用的机器连接")?;
            ensure!(source.codex_home_id.is_none(), "请通过 Codex 登录连接账户");
            ensure!(
                source.account_id.is_empty() || source.account_id == account_id,
                "此目录已连接其他账户；请在高级设置添加独立目录，或选择已有账户"
            );
            source.account_id = account_id.clone();
        }
        let settings = self.store.patch_settings(
            &serde_json::to_value(base)?,
            &serde_json::to_value(next)?,
            false,
        )?;
        Ok(json!({"accountId":account_id,"settings":settings}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Engine) {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::open(root.path()).unwrap();
        engine
            .store
            .save_settings(&Settings {
                hosts: vec![Host {
                    id: "server".into(),
                    name: "Server".into(),
                    target: "fixture.invalid".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        (root, engine)
    }

    #[test]
    fn directory_removal_preserves_credentials_and_disconnects_account() {
        let (root, mut engine) = fixture();
        let path = root.path().join("codex");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("auth.json"), "fixture credential").unwrap();
        let mut settings = engine.store.settings().unwrap();
        settings.sources.push(Source {
            id: "managed".into(),
            path: path.display().to_string(),
            codex_home_id: Some("home".into()),
            ..Default::default()
        });
        settings.accounts.push(Account {
            id: "a".into(),
            name: "A".into(),
            quota_profile_id: Some("home:profile".into()),
            quota_source_id: Some("managed".into()),
            ..Default::default()
        });
        engine.store.save_auth_settings(&settings).unwrap();
        engine
            .store
            .db
            .execute(
                "INSERT INTO events VALUES('event','codex','a','fixture',1,'{}',0,0,'fixture')",
                [],
            )
            .unwrap();
        engine
            .store
            .db
            .execute("INSERT INTO event_sources VALUES('event','managed')", [])
            .unwrap();
        let saved = engine
            .call("sources.remove", json!({"sourceId":"managed"}))
            .unwrap();
        assert!(saved["sources"].as_array().unwrap().is_empty());
        let a = &saved["accounts"][0];
        assert_eq!(a["id"], "a");
        assert!(a["connections"].as_array().unwrap().is_empty());
        assert!(a["quotaProfileId"].is_null());
        assert!(a["quotaSourceId"].is_null());
        assert_eq!(
            engine
                .store
                .db
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            engine
                .store
                .db
                .query_row("SELECT COUNT(*) FROM event_sources", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(path.join("auth.json")).unwrap(),
            "fixture credential"
        );
    }

    #[test]
    fn directory_move_detaches_managed_identity_and_guards_tasks() {
        let (_root, mut engine) = fixture();
        engine
            .call(
                "agents.set",
                json!({"provider":"codex","enabled":true,"machineIds":["local"]}),
            )
            .unwrap();
        let mut settings = engine.store.settings().unwrap();
        settings.sources[0].codex_home_id = Some("home".into());
        engine.store.save_auth_settings(&settings).unwrap();
        let old = engine.store.settings().unwrap().sources[0].clone();
        let mut moved = old.clone();
        moved.path = "~/other-codex".into();
        let task = crate::wakeups::Record {
            task: crate::wakeups::Task {
                id: "t".into(),
                name: "Keep me".into(),
                source_id: old.id.clone(),
                ..Default::default()
            },
            deployment: None,
        };
        engine.store.save_wakeup(&task).unwrap();
        assert!(
            engine
                .call("sources.remove", json!({"sourceId":old.id}))
                .unwrap_err()
                .to_string()
                .contains("Keep me")
        );
        assert!(
            engine
                .call(
                    "sources.configure",
                    json!({"source":moved,"baseSource":old})
                )
                .is_err()
        );
        engine
            .store
            .db
            .execute("DELETE FROM kv WHERE key='wakeup:t'", [])
            .unwrap();
        let saved = engine
            .call(
                "sources.configure",
                json!({"source":moved,"baseSource":old}),
            )
            .unwrap();
        assert_ne!(saved["sources"][0]["id"], old.id);
        assert!(saved["sources"][0]["codexHomeId"].is_null());
        let id = saved["sources"][0]["id"].as_str().unwrap();
        let saved = engine
            .call("sources.remove", json!({"sourceId":id}))
            .unwrap();
        assert!(
            saved["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["provider"] == "codex")
                .unwrap()["machineIds"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn directory_update_accepts_omitted_optional_fields_and_rejects_stale_edits() {
        let (_root, mut engine) = fixture();
        engine
            .call(
                "agents.set",
                json!({"provider":"claude","enabled":true,"machineIds":["local"]}),
            )
            .unwrap();
        let source = engine.store.settings().unwrap().sources[0].clone();
        let mut base = serde_json::to_value(&source).unwrap();
        base.as_object_mut().unwrap().retain(|_, v| !v.is_null());
        let mut changed = source.clone();
        changed.path = "~/claude-2".into();
        let saved = engine
            .call(
                "sources.configure",
                json!({"source":changed,"baseSource":base}),
            )
            .unwrap();
        assert_eq!(saved["sources"][0]["id"], source.id);
        assert!(
            engine
                .call(
                    "sources.configure",
                    json!({"source":source,"baseSource":base})
                )
                .is_err()
        );
    }

    #[test]
    fn machines_toggle_without_changing_source_identity_or_account() {
        let (_root, mut engine) = fixture();
        let first = engine
            .call(
                "agents.set",
                json!({"provider":"claude","enabled":true,"machineIds":["local","server"]}),
            )
            .unwrap();
        assert_eq!(first["sources"].as_array().unwrap().len(), 2);
        let ids: Vec<_> = first["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].clone())
            .collect();
        engine
            .call(
                "accounts.connect",
                json!({"provider":"claude","name":"Shared","sourceIds":ids}),
            )
            .unwrap();
        let off = engine
            .call("agents.set", json!({"provider":"claude","enabled":false}))
            .unwrap();
        assert!(
            off["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["enabled"] == false)
        );
        assert_eq!(
            off["accounts"][0]["connections"].as_array().unwrap().len(),
            2
        );
        let on = engine
            .call("agents.set", json!({"provider":"claude","enabled":true}))
            .unwrap();
        assert_eq!(
            on["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].clone())
                .collect::<Vec<_>>(),
            ids
        );
        assert!(
            on["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["enabled"] == true)
        );
        let empty = engine
            .call("agents.set", json!({"provider":"claude","machineIds":[]}))
            .unwrap();
        assert!(
            empty["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["enabled"] == false)
        );
        assert_eq!(empty["accounts"][0]["id"], on["accounts"][0]["id"]);
    }

    #[test]
    fn patch_preserves_other_windows_and_rejects_conflicts_atomically() {
        let (_root, mut engine) = fixture();
        let base = engine.call("settings.get", json!({})).unwrap();
        let mut first = base.clone();
        first["hosts"][0]["name"] = json!("Renamed");
        engine
            .call("settings.patch", json!({"base":base,"settings":first}))
            .unwrap();
        let mut second = base.clone();
        second["refreshSeconds"] = json!(600);
        let merged = engine
            .call("settings.patch", json!({"base":base,"settings":second}))
            .unwrap();
        assert_eq!(merged["hosts"][0]["name"], "Renamed");
        assert_eq!(merged["refreshSeconds"], 600);
        second["hosts"][0]["name"] = json!("Conflicting");
        second["refreshSeconds"] = json!(900);
        assert!(
            engine
                .call("settings.patch", json!({"base":base,"settings":second}))
                .unwrap_err()
                .to_string()
                .contains("其他窗口")
        );
        assert_eq!(engine.call("settings.get", json!({})).unwrap(), merged);
    }

    #[test]
    fn migration_preserves_archives_and_multiple_profiles_share_one_quota_card() {
        let (_root, engine) = fixture();
        let mut s = Settings {
            version: 2,
            sources: vec![
                Source {
                    id: "one".into(),
                    path: "/fixture/one".into(),
                    codex_home_id: Some("home1".into()),
                    ..Default::default()
                },
                Source {
                    id: "two".into(),
                    path: "/fixture/two".into(),
                    codex_home_id: Some("home2".into()),
                    ..Default::default()
                },
            ],
            accounts: vec![
                Account {
                    id: "a".into(),
                    name: "One identity".into(),
                    quota_profile_id: Some("home1:profile1".into()),
                    connections: vec![AccountConnection {
                        source_id: "two".into(),
                        profile_id: Some("home2:profile2".into()),
                    }],
                    ..Default::default()
                },
                Account {
                    id: "archived".into(),
                    name: "Archived".into(),
                    archived: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        s.migrate();
        assert!(s.accounts[1].archived);
        assert_eq!(s.accounts[0].profile_refs().len(), 2);
        engine.store.save_auth_settings(&s).unwrap();
        let dashboard = engine.store.dashboard(&Filter::default()).unwrap();
        assert_eq!(dashboard.quotas.len(), 1);
        assert_eq!(dashboard.quotas[0].account_id, "a");
        let base = serde_json::to_value(engine.store.settings().unwrap()).unwrap();
        let mut edited = base.clone();
        edited["accounts"][0]["name"] = json!("Renamed");
        edited["accounts"][0]["quotaSourceId"] = json!("two");
        engine.store.patch_settings(&base, &edited, false).unwrap();
        edited["sources"][0]["path"] = json!("/new-home");
        assert!(
            engine
                .store
                .patch_settings(&base, &edited, false)
                .unwrap_err()
                .to_string()
                .contains("受管理")
        );
        assert_eq!(
            engine.store.settings().unwrap().sources[0].path,
            "/fixture/one"
        );
    }

    #[test]
    fn invalid_machine_or_reassigning_a_login_leaves_settings_unchanged() {
        let (_root, mut engine) = fixture();
        assert!(
            engine
                .call(
                    "agents.set",
                    json!({"provider":"deepseek","enabled":true,"machineIds":["server"]})
                )
                .is_err()
        );
        let initial = engine
            .call(
                "agents.set",
                json!({"provider":"claude","enabled":true,"machineIds":["local"]}),
            )
            .unwrap();
        let id = &initial["sources"][0]["id"];
        let result = engine
            .call(
                "accounts.connect",
                json!({"provider":"claude","name":"First","sourceIds":[id]}),
            )
            .unwrap();
        assert!(
            engine
                .call(
                    "accounts.connect",
                    json!({"provider":"claude","name":"Second","sourceIds":[id]})
                )
                .is_err()
        );
        assert_eq!(
            engine.call("settings.get", json!({})).unwrap(),
            result["settings"]
        );
    }
}
