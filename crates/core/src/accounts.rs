//! Account/device configuration, lightweight observations and explicit account removal.
use crate::{Engine, models::*, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};

pub fn default_name(provider: &str) -> String {
    match provider {
        "codex" => "Codex · 待登录",
        "claude" => "Claude Code 账户",
        "antigravity" => "Antigravity 账户",
        "deepseek" => "DeepSeek 账户",
        _ => "自定义 Agent 账户",
    }
    .into()
}

pub fn pre_command(settings: &Settings, account: Option<&Account>, source: &Source) -> String {
    let machine = source.host_id.as_deref().unwrap_or("local");
    if let Some(value) =
        account.and_then(|a| a.device_settings.iter().find(|d| d.machine_id == machine))
    {
        return value.pre_command.clone(); // An explicit empty string suppresses legacy inheritance.
    }
    if !source.quota_pre_command.trim().is_empty() {
        return source.quota_pre_command.clone();
    }
    source
        .host_id
        .as_ref()
        .and_then(|id| settings.hosts.iter().find(|h| &h.id == id))
        .map(|h| h.pre_command.clone())
        .unwrap_or_default()
}
pub(crate) fn key(account: &Account) -> String {
    format!("{}:{}", account.provider, account.id)
}
fn cache_key(account: &Account, source: &Source) -> String {
    format!("account-status:{}:{}", key(account), source.id)
}
fn fingerprint(settings: &Settings, account: &Account, source: &Source) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    json!([
        source,
        account.identity_key,
        account.profile_refs(),
        pre_command(settings, Some(account), source),
        source
            .host_id
            .as_ref()
            .and_then(|id| settings.hosts.iter().find(|h| &h.id == id))
    ])
    .to_string()
    .hash(&mut hash);
    hash.finish()
}
fn get(store: &Store, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
        .optional()?;
    raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
}
fn put(store: &Store, key: &str, value: &Value) -> Result<()> {
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![key, value.to_string()],
    )?;
    Ok(())
}
pub fn task_uses(account: &Account, settings: &Settings, record: &crate::wakeups::Record) -> bool {
    let task_matches = |task: &crate::wakeups::Task| {
        task.codex_profile_id
            .as_deref()
            .is_some_and(|p| account.has_profile(p))
            || (task.codex_profile_id.is_none()
                && settings.sources.iter().any(|s| {
                    s.id == task.source_id
                        && s.provider == account.provider
                        && s.account_id == account.id
                }))
    };
    task_matches(&record.task)
        || record.deployment.as_ref().is_some_and(|d| {
            task_matches(&d.task)
                || (d.source.provider == account.provider && d.source.account_id == account.id)
        })
}
pub fn lock_tasks(store: &Store) -> Result<crate::file_lock::FileLock> {
    let root = std::path::Path::new(store.db.path().context("数据目录不可用")?)
        .parent()
        .context("数据目录不可用")?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("account-tasks.lock"))?;
    crate::file_lock::FileLock::try_exclusive(file)
        .map_err(|_| anyhow::anyhow!("有账户任务正在部署或删除，请稍后重试"))
}
pub fn ensure_not_deleting(store: &Store, account: &Account) -> Result<()> {
    ensure!(
        get(store, &format!("account-deleting:{}", key(account)))?.is_none(),
        "此账户正在删除，请等待清理完成"
    );
    Ok(())
}
fn public_task(record: &crate::wakeups::Record) -> Value {
    json!({"id":record.task.id,"name":record.task.name,"deployed":record.deployment.is_some(),
        "machine":record.deployment.as_ref().and_then(|d| d.host.as_ref()).map(|h| if h.name.is_empty() { &h.target } else { &h.name }).map(String::as_str).unwrap_or("本机"),
        "root":record.deployment.as_ref().map(|d| &d.root)})
}
impl Engine {
    pub(crate) fn account_call(&mut self, method: &str, params: Value) -> Result<Value> {
        if method == "accounts.cleanup.list" {
            let mut stmt = self
                .store
                .db
                .prepare("SELECT value FROM kv WHERE key LIKE 'account-cleanup:%' ORDER BY key")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(json!(
                rows.iter()
                    .map(|r| serde_json::from_str::<Value>(r))
                    .collect::<std::result::Result<Vec<_>, _>>()?
            ));
        }
        let settings = self.store.settings()?;
        if method == "accounts.create" {
            let provider = params["provider"].as_str().context("请选择 Agent")?;
            ensure!(
                settings
                    .agents
                    .iter()
                    .any(|a| a.provider == provider && a.enabled),
                "请先启用此 Agent"
            );
            let name = params["name"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let mut next = settings.clone();
            let id = crate::codex_auth::files::new_id();
            next.accounts.push(Account {
                id: id.clone(),
                name: name
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_name(provider)),
                pending_name: provider == "codex" && name.is_none(),
                provider: provider.into(),
                ..Default::default()
            });
            let saved = self
                .store
                .patch_settings(&json!(settings), &json!(next), false)?;
            return Ok(json!({"settings":saved,"accountKey":format!("{provider}:{id}")}));
        }
        if method == "accounts.status.get" {
            let mut rows = vec![];
            for account in &settings.accounts {
                for source in settings
                    .sources
                    .iter()
                    .filter(|s| s.provider == account.provider)
                {
                    let mut row = get(&self.store, &cache_key(account, source))?.unwrap_or_else(
                        || json!({"current":null,"credential":null,"checkedAt":null}),
                    );
                    if row["fingerprint"] != json!(fingerprint(&settings, account, source)) {
                        row = json!({"current":null,"credential":null,"checkedAt":null});
                    }
                    row["accountKey"] = json!(key(account));
                    row["sourceId"] = json!(source.id);
                    row["machineId"] = json!(source.host_id.as_deref().unwrap_or("local"));
                    row.as_object_mut().unwrap().remove("fingerprint");
                    rows.push(row);
                }
            }
            return Ok(json!(rows));
        }
        if method == "accounts.deployments.sync" {
            return self.sync_account_deployments();
        }
        let account_key = params["accountKey"].as_str().context("请选择账户")?;
        if method == "accounts.delete"
            && !settings.accounts.iter().any(|a| key(a) == account_key)
            && get(&self.store, &format!("deleted-account:{account_key}"))?.is_some()
        {
            return Ok(json!({"deleted":true,"settings":settings}));
        }
        let account = settings
            .accounts
            .iter()
            .find(|a| key(a) == account_key)
            .context("账户已删除，请刷新")?
            .clone();
        if method == "accounts.status.refresh" {
            let source = settings
                .sources
                .iter()
                .find(|s| params["sourceId"] == s.id && s.provider == account.provider)
                .context("设备目录不存在")?;
            ensure!(
                source.enabled
                    && source
                        .host_id
                        .as_ref()
                        .is_none_or(|id| settings.hosts.iter().any(|h| &h.id == id && h.enabled)),
                "设备已暂停"
            );
            let stamp = fingerprint(&settings, &account, source);
            let result = self.inspect_account_device(&settings, &account, source);
            // Observations made with old settings must never overwrite a new device's state.
            let latest = self.store.settings()?;
            let unchanged = latest
                .accounts
                .iter()
                .find(|a| key(a) == account_key)
                .zip(latest.sources.iter().find(|s| s.id == source.id))
                .is_some_and(|(a, s)| fingerprint(&latest, a, s) == stamp);
            ensure!(unchanged, "设备配置已变化，请重新检查");
            let mut row = match result {
                Ok(v) => v,
                Err(e) => {
                    let mut old = get(&self.store, &cache_key(&account, source))?
                        .filter(|v| v["fingerprint"] == json!(stamp))
                        .unwrap_or_else(
                            || json!({"current":null,"credential":null,"checkedAt":null}),
                        );
                    old["error"] = json!(e.to_string());
                    old
                }
            };
            row["attemptedAt"] = json!(now());
            row["fingerprint"] = json!(stamp);
            put(&self.store, &cache_key(&account, source), &row)?;
            row["accountKey"] = json!(account_key);
            row["sourceId"] = json!(source.id);
            row["machineId"] = json!(source.host_id.as_deref().unwrap_or("local"));
            row.as_object_mut().unwrap().remove("fingerprint");
            return Ok(row);
        }
        let tasks = self
            .store
            .wakeups()?
            .into_iter()
            .filter(|r| task_uses(&account, &settings, r))
            .collect::<Vec<_>>();
        if method == "accounts.deletion.preview" {
            return Ok(
                json!({"accountKey":account_key,"name":account.name,"tasks":tasks.iter().map(public_task).collect::<Vec<_>>() }),
            );
        }
        ensure!(method == "accounts.delete", "未知账户操作");
        ensure!(account.archived, "请先归档账户再删除");
        // Serialize deletion against new deployments, without holding a SQL lock over SSH.
        let _task_lock = lock_tasks(&self.store)?;
        let marker = format!("account-deleting:{account_key}");
        put(&self.store, &marker, &json!({"startedAt":now()}))?;
        let result =
            self.delete_archived_account(&account, params["force"].as_bool().unwrap_or(false));
        self.store
            .db
            .execute("DELETE FROM kv WHERE key=?1", [marker])?;
        result
    }

    fn inspect_account_device(
        &self,
        settings: &Settings,
        account: &Account,
        source: &Source,
    ) -> Result<Value> {
        if source.provider == "codex" {
            let info = crate::codex_auth::backend(
                source,
                settings,
                "status",
                &json!({"accountId":account.id}),
            )?;
            if info["storageMode"] != "file" {
                return Ok(
                    json!({"current":null,"credential":null,"checkedAt":now(),"note":"认证存储不支持状态识别"}),
                );
            }
            let profiles = info["profiles"].as_array().cloned().unwrap_or_default();
            let matching = profiles
                .iter()
                .filter(|p| {
                    account
                        .identity_key
                        .as_deref()
                        .is_some_and(|k| p["identityKey"] == k)
                        || source.codex_home_id.as_ref().is_some_and(|h| {
                            p["id"]
                                .as_str()
                                .is_some_and(|id| account.has_profile(&format!("{h}:{id}")))
                        })
                })
                .collect::<Vec<_>>();
            let identity = account.identity_key.clone().or_else(|| {
                matching
                    .first()
                    .and_then(|p| p["identityKey"].as_str().map(str::to_owned))
            });
            let current = identity.as_deref().map(|id| info["currentIdentity"] == id);
            let credential = identity.as_deref().map(|id| {
                (info["currentIdentity"] == id && info["currentCredential"] == true)
                    || matching.iter().any(|p| p["credential"] == true)
            });
            return Ok(
                json!({"current":current,"credential":credential,"checkedAt":now(),"note": if identity.is_none() { "尚未验证账户身份，请读取当前登录或授权" } else { "" }}),
            );
        }
        // Other providers cannot safely infer the identity of a machine's current login.
        let credential = if !account.uses_source(source) {
            Some(false)
        } else if source.provider == "deepseek" && source.host_id.is_none() {
            Some(crate::store::expand(&source.path).is_file())
        } else if source.provider == "claude"
            && source.host_id.is_none()
            && crate::store::expand(&source.path)
                .join(".credentials.json")
                .is_file()
        {
            Some(true)
        } else {
            None
        };
        Ok(
            json!({"current":null,"credential":credential,"checkedAt":now(),"note":"此 Agent 按目录连接；当前账户状态需由 Agent 确认"}),
        )
    }

    fn delete_archived_account(&mut self, account: &Account, force: bool) -> Result<Value> {
        let settings = self.store.settings()?;
        let tasks = self
            .store
            .wakeups()?
            .into_iter()
            .filter(|r| task_uses(account, &settings, r))
            .collect::<Vec<_>>();
        let mut failed = vec![];
        let mut abandoned = vec![];
        let mut forced_ids = vec![];
        for record in tasks {
            if record.deployment.is_some() {
                if force {
                    abandoned.push(public_task(&record));
                } else {
                    self.progress(&format!("正在停止并移除任务：{}", record.task.name));
                    if let Err(e) =
                        self.wakeup_call_unlocked("wakeups.remove", json!({"id":record.task.id}))
                    {
                        let mut failure = public_task(&record);
                        failure["error"] = json!(e.to_string());
                        failed.push(failure);
                        continue;
                    }
                }
            }
            if force {
                forced_ids.push(record.task.id);
            } else {
                self.wakeup_call_unlocked("wakeups.delete", json!({"id":record.task.id}))?;
            }
        }
        if !failed.is_empty() {
            return Ok(json!({"deleted":false,"failedTasks":failed,"canForce":true}));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.store.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let base = self.store.settings()?;
        ensure!(
            base.accounts
                .iter()
                .any(|a| key(a) == key(account) && a.archived),
            "账户状态已变化，请重新确认删除"
        );
        for id in forced_ids {
            self.store
                .db
                .execute("DELETE FROM kv WHERE key=?1", [format!("wakeup:{id}")])?;
        }
        ensure!(
            !self
                .store
                .wakeups()?
                .iter()
                .any(|r| task_uses(account, &base, r)),
            "出现新的关联任务，请重试清理"
        );
        let mut next = base.clone();
        next.accounts.retain(|a| key(a) != key(account));
        for source in &mut next.sources {
            if source.provider == account.provider && source.account_id == account.id {
                source.account_id.clear();
            }
        }
        put(
            &self.store,
            &format!("deleted-account:{}", key(account)),
            &json!({"id":account.id,"provider":account.provider,"name":account.name,"deletedAt":now()}),
        )?;
        if !abandoned.is_empty() {
            put(
                &self.store,
                &format!("account-cleanup:{}", key(account)),
                &json!({"accountKey":key(account),"name":account.name,"deletedAt":now(),"tasks":abandoned,"warning":"已解除本地管理；这些设备上的远端任务可能仍在运行"}),
            )?;
        }
        self.store.save_auth_settings(&next)?;
        let saved = self.store.settings()?;
        tx.commit()?;
        Ok(json!({"deleted":true,"settings":saved,"abandonedTasks":abandoned}))
    }

    fn sync_account_deployments(&mut self) -> Result<Value> {
        let mut results = vec![];
        for record in self.store.wakeups()? {
            let Some(deployment) = &record.deployment else {
                continue;
            };
            if deployment.state != "settings-pending" {
                continue;
            }
            let enabled = deployment.enabled;
            let id = &record.task.id;
            let result = self.wakeup_call(
                "wakeups.deploy",
                json!({"id":id,"preserveEnabled":enabled,"useDeployedTask":true}),
            );
            let result = result.and_then(|value| {
                ensure!(
                    value["deployment"]["state"] != "settings-pending",
                    "同步期间配置再次变化，请重试同步"
                );
                Ok(value)
            });
            if let Err(e) = result {
                if let Ok(mut current) = self.store.wakeup(id) {
                    if let Some(d) = &mut current.deployment {
                        d.state = "settings-pending".into();
                    }
                    self.store.save_wakeup(&current)?;
                }
                let mut failure = public_task(&record);
                failure["error"] = json!(e.to_string());
                results.push(failure);
            }
        }
        Ok(json!({"failedTasks":results}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wakeups::{Deployment, Record, Task};
    fn fixture() -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            account_id: "a".into(),
            path: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        engine
            .store
            .save_settings(&Settings {
                sources: vec![source],
                accounts: vec![Account {
                    id: "a".into(),
                    name: "Preserved account".into(),
                    archived: true,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        (dir, engine)
    }
    #[test]
    fn delete_preserves_history_credentials_and_deduplication_namespace() {
        let (dir, mut e) = fixture();
        let credential = dir.path().join("auth.json");
        std::fs::write(&credential, "FAKE-SECRET").unwrap();
        e.store
            .db
            .execute(
                "INSERT INTO events VALUES('event','codex','a','m',1,'{}',2,3,NULL)",
                [],
            )
            .unwrap();
        let base = e.store.settings().unwrap();
        let mut removed = base.clone();
        removed.accounts.clear();
        removed.sources[0].account_id.clear();
        assert!(e.store.save_settings(&removed).is_err());
        let result = e
            .account_call("accounts.delete", json!({"accountKey":"codex:a"}))
            .unwrap();
        assert_eq!(result["deleted"], true);
        let settings = e.store.settings().unwrap();
        assert!(settings.accounts.is_empty());
        assert_eq!(settings.deleted_accounts[0].name, "Preserved account");
        assert_eq!(settings.sources[0].account_id, "");
        assert_eq!(std::fs::read_to_string(credential).unwrap(), "FAKE-SECRET");
        assert_eq!(
            e.store
                .db
                .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            e.store
                .db
                .query_row(
                    "SELECT account_id FROM source_identities WHERE source_id='s'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "a"
        );
        // A stale client's edit cannot resurrect the deleted account.
        let mut stale = base.clone();
        stale.accounts[0].name = "changed".into();
        assert!(
            e.store
                .patch_settings(&json!(base), &json!(stale), false)
                .is_err()
        );
    }
    #[test]
    fn normal_deletion_removes_drafts_and_force_records_unreachable_deployments() {
        let (_dir, mut e) = fixture();
        let settings = e.store.settings().unwrap();
        let task = Task {
            id: "job".into(),
            name: "abandoned job".into(),
            source_id: "s".into(),
            ..Default::default()
        };
        let draft = Task {
            id: "draft".into(),
            ..task.clone()
        };
        e.store
            .save_wakeup(&Record {
                task: draft,
                deployment: None,
            })
            .unwrap();
        let record = Record {
            task: task.clone(),
            deployment: Some(Deployment {
                task,
                source: settings.sources[0].clone(),
                host: Some(Host {
                    id: "gone".into(),
                    name: "Abandoned machine".into(),
                    target: "".into(),
                    ..Default::default()
                }),
                root: "~/fixture/jobs".into(),
                deployed_at: 1,
                enabled: true,
                state: "deployed".into(),
                timezone: "UTC".into(),
            }),
        };
        e.store.save_wakeup(&record).unwrap();
        let result = e
            .account_call("accounts.delete", json!({"accountKey":"codex:a"}))
            .unwrap();
        assert_eq!(result["deleted"], false);
        assert_eq!(result["failedTasks"].as_array().unwrap().len(), 1);
        assert_eq!(e.store.wakeups().unwrap().len(), 1);
        assert_eq!(e.store.settings().unwrap().accounts.len(), 1);
        let result = e
            .account_call(
                "accounts.delete",
                json!({"accountKey":"codex:a","force":true}),
            )
            .unwrap();
        assert_eq!(result["deleted"], true);
        assert!(e.store.wakeups().unwrap().is_empty());
        let audit = e.account_call("accounts.cleanup.list", json!({})).unwrap();
        assert_eq!(audit[0]["tasks"][0]["root"], "~/fixture/jobs");
        assert!(!audit.to_string().contains("passwordRef"));
    }
    #[test]
    fn active_account_cannot_be_deleted_and_deploy_lock_prevents_races() {
        let (_dir, mut e) = fixture();
        let lock = lock_tasks(&e.store).unwrap();
        assert!(
            e.account_call(
                "accounts.delete",
                json!({"accountKey":"codex:a","force":true})
            )
            .is_err()
        );
        drop(lock);
        let mut s = e.store.settings().unwrap();
        s.accounts[0].archived = false;
        e.store.save_settings(&s).unwrap();
        assert!(
            e.account_call(
                "accounts.delete",
                json!({"accountKey":"codex:a","force":true})
            )
            .is_err()
        );
    }
    #[test]
    fn account_device_commands_are_independent_and_empty_is_explicit() {
        let mut s = Settings {
            version: 3,
            hosts: vec![Host {
                id: "remote".into(),
                pre_command: "legacy host".into(),
                ..Default::default()
            }],
            sources: vec![Source {
                id: "s".into(),
                account_id: "a".into(),
                host_id: Some("remote".into()),
                quota_pre_command: "legacy source".into(),
                ..Default::default()
            }],
            accounts: vec![Account {
                id: "a".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        s.migrate();
        assert_eq!(
            s.accounts[0].device_settings[0].pre_command,
            "legacy source"
        );
        s.accounts[0].device_settings[0].pre_command.clear();
        assert_eq!(pre_command(&s, Some(&s.accounts[0]), &s.sources[0]), "");
        let other = Account {
            device_settings: vec![AccountDeviceSettings {
                machine_id: "remote".into(),
                pre_command: "other".into(),
            }],
            ..Default::default()
        };
        assert_eq!(pre_command(&s, Some(&other), &s.sources[0]), "other");
        let host = Host {
            target: "fixture.invalid".into(),
            pre_command: "NEVER-SYNC".into(),
            ..Default::default()
        };
        let command = crate::ssh::command(&host, "history").unwrap();
        assert!(
            !command
                .get_args()
                .last()
                .unwrap()
                .to_string_lossy()
                .contains("NEVER-SYNC")
        );
        let command = crate::ssh::account_command(&host, "quota").unwrap();
        assert!(
            command
                .get_args()
                .last()
                .unwrap()
                .to_string_lossy()
                .contains("NEVER-SYNC")
        );
    }
    #[test]
    fn conflicting_legacy_directories_remain_distinct_until_explicit_device_save() {
        let mut s = Settings {
            version: 3,
            accounts: vec![Account {
                id: "a".into(),
                ..Default::default()
            }],
            sources: vec![
                Source {
                    id: "s1".into(),
                    account_id: "a".into(),
                    host_id: Some("server".into()),
                    quota_pre_command: "first".into(),
                    ..Default::default()
                },
                Source {
                    id: "s2".into(),
                    account_id: "a".into(),
                    host_id: Some("server".into()),
                    quota_pre_command: "second".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        s.migrate();
        assert!(s.accounts[0].device_settings.is_empty());
        assert_eq!(
            pre_command(&s, Some(&s.accounts[0]), &s.sources[0]),
            "first"
        );
        assert_eq!(
            pre_command(&s, Some(&s.accounts[0]), &s.sources[1]),
            "second"
        );
    }
    #[test]
    fn lightweight_status_distinguishes_current_from_stored_and_invalidates_old_config() {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        let (dir, mut e) = fixture();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let auth = |user: &str| {
            let token=URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth":{"chatgpt_user_id":user,"chatgpt_account_id":"workspace"}}).to_string());
            json!({"auth_mode":"chatgpt","tokens":{"access_token":format!("head.{token}.tail"),"refresh_token":"FIXTURE","account_id":"workspace"}})
        };
        let one = auth("one");
        let two = auth("two");
        crate::codex_auth::files::save_profile(&canonical, "one", "One", &one).unwrap();
        crate::codex_auth::files::save_profile(&canonical, "two", "Two", &two).unwrap();
        crate::codex_auth::files::atomic(&canonical.join("auth.json"), &one).unwrap();
        let mut settings = e.store.settings().unwrap();
        settings.sources[0].path = canonical.to_string_lossy().into();
        settings.sources[0].codex_home_id = Some("home".into());
        settings.sources[0].account_id.clear();
        settings.sources[0].codex_binary = "missing-fixture-cli".into();
        settings.accounts[0].identity_key =
            Some(crate::codex_auth::files::identity(&two).unwrap().key);
        settings.accounts[0].quota_profile_id = Some("home:two".into());
        e.store.save_auth_settings(&settings).unwrap();
        let result = e
            .account_call(
                "accounts.status.refresh",
                json!({"accountKey":"codex:a","sourceId":"s"}),
            )
            .unwrap();
        assert_eq!(result["current"], false);
        assert_eq!(result["credential"], true);
        assert!(!result.to_string().contains("FIXTURE"));
        let mut settings = e.store.settings().unwrap();
        settings.accounts[0]
            .device_settings
            .push(AccountDeviceSettings {
                machine_id: "local".into(),
                pre_command: "changed".into(),
            });
        e.store.save_settings(&settings).unwrap();
        let cached = e.account_call("accounts.status.get", json!({})).unwrap();
        assert!(cached[0]["current"].is_null());
    }
}
