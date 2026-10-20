//! Evidence used only for subscription valuation. Never relabel shared usage.
use crate::{models::*, store::Store};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

pub(crate) const VERSION: u32 = 1;
const MAX_GAP: i64 = 300;

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS codex_session_creators(source_id TEXT NOT NULL,session_id TEXT NOT NULL,identity_key TEXT NOT NULL,created_at INTEGER NOT NULL,conflict INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(source_id,session_id));
        CREATE TABLE IF NOT EXISTS codex_account_timeline(id INTEGER PRIMARY KEY,source_id TEXT NOT NULL,stamp INTEGER NOT NULL,identity_key TEXT NOT NULL,kind TEXT NOT NULL,uncertain_from INTEGER);
        CREATE INDEX IF NOT EXISTS codex_account_time ON codex_account_timeline(source_id,stamp,id);
        CREATE TABLE IF NOT EXISTS codex_request_limits(source_id TEXT NOT NULL,request_id TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(source_id,request_id));")?;
    // Preserve original amounts and segments; only remove unsupported legacy evidence
    // from the live pool. The rolling projection's update trigger marks these dirty.
    db.execute_batch("CREATE INDEX IF NOT EXISTS codex_unverified_estimates ON estimates(account_key) WHERE substr(account_key,1,6)='codex:' AND json_extract(payload,'$.samplingMode')='automatic' AND COALESCE(json_extract(payload,'$.calculationVersion'),0)<4 AND json_extract(payload,'$.rollingExcludedReason') IS NULL;")?;
    db.execute("UPDATE estimates SET status='completed',payload=json_set(payload,'$.status','completed','$.endedAt',COALESCE(json_extract(payload,'$.endedAt'),json_extract(payload,'$.checkpointAt')),'$.rollingExcludedReason','旧样本缺少账户联合归属证据，仅保留历史，不参与当前滚动估值') WHERE substr(account_key,1,6)='codex:' AND json_extract(payload,'$.samplingMode')='automatic' AND COALESCE(json_extract(payload,'$.calculationVersion'),0)<4 AND json_extract(payload,'$.rollingExcludedReason') IS NULL", [])?;
    Ok(())
}

fn evidence_scope(settings: &Settings, source: &Source) -> String {
    let home = crate::codex_auth::home_path(&source.path);
    let home = if source.host_id.is_none() {
        std::fs::canonicalize(crate::store::expand(&home))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(home)
    } else {
        home
    };
    let host = settings
        .hosts
        .iter()
        .find(|h| Some(&h.id) == source.host_id.as_ref())
        .map(|h| json!([h.target, h.port, h.username]));
    hash(&json!([home, host]).to_string())
}

impl Store {
    /// Both local and SSH readers pass through the Rust parser before committing.
    pub(crate) fn record_codex_history(
        &self,
        source: &Source,
        state: &ParseState,
        row: &Value,
        event: Option<&UsageEvent>,
    ) -> Result<()> {
        if source.provider != "codex" {
            return Ok(());
        }
        if row["type"] == "session_meta" {
            let p = &row["payload"];
            let raw_identity = p["creator_user_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .zip(p["creator_account_id"].as_str().filter(|s| !s.is_empty()));
            let identity = raw_identity
                .map(|(user, account)| hash(&format!("{user}\0{account}")))
                .or_else(|| {
                    p["creator_identity_key"]
                        .as_str()
                        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                        .map(str::to_owned)
                });
            if let Some(identity) = identity {
                let stamp = timestamp(&p["timestamp"])
                    .or_else(|| timestamp(&row["timestamp"]))
                    .unwrap_or(0);
                self.db.execute("INSERT INTO codex_session_creators VALUES(?1,?2,?3,?4,0) ON CONFLICT(source_id,session_id) DO UPDATE SET conflict=codex_session_creators.conflict OR codex_session_creators.identity_key<>excluded.identity_key", params![source.id,state.session_id,identity,stamp])?;
            }
        }
        if let Some(e) = event
            && e.billing.evidence == "codex-response-rate-limits"
        {
            let q = crate::usage::codex_quota(
                source,
                &row["payload"]["rate_limits"],
                e.timestamp,
                "log",
            );
            self.db.execute("INSERT INTO codex_request_limits VALUES(?1,?2,?3) ON CONFLICT(source_id,request_id) DO UPDATE SET payload=excluded.payload", params![source.id,e.import_identity.as_deref().unwrap_or(&e.id),serde_json::to_string(&q)?])?;
        }
        Ok(())
    }

    /// Source-clock timestamps are compared with that source's rollout timestamps.
    /// A changed identity observed by polling creates a gap, not a guessed switch instant.
    pub(crate) fn record_codex_identity(
        &self,
        source: &Source,
        identity: &str,
        stamp: i64,
        kind: &str,
        gap: Option<i64>,
    ) -> Result<()> {
        if source.provider != "codex" || stamp <= 0 {
            return Ok(());
        }
        let prior: Option<(i64, String)> = self.db.query_row("SELECT stamp,identity_key FROM codex_account_timeline WHERE source_id=?1 ORDER BY stamp DESC,id DESC LIMIT 1", [&source.id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if prior
            .as_ref()
            .is_some_and(|(t, k)| *t == stamp && k == identity)
        {
            return Ok(());
        }
        // Never let a late status response insert an apparent switch into the past.
        if prior.as_ref().is_some_and(|(t, _)| *t > stamp) {
            return Ok(());
        }
        let uncertain = gap.or_else(|| {
            prior.as_ref().and_then(|(t, k)| {
                (kind != "switchBefore" && (k != identity || stamp - t > MAX_GAP)).then_some(*t)
            })
        });
        self.db.execute("INSERT INTO codex_account_timeline(source_id,stamp,identity_key,kind,uncertain_from) VALUES(?1,?2,?3,?4,?5)", params![source.id,stamp,identity,kind,uncertain])?;
        Ok(())
    }

    pub(crate) fn record_codex_observation(
        &self,
        settings: &Settings,
        source: &Source,
        identity: &str,
        stamp: i64,
    ) -> Result<()> {
        let tx = self
            .db
            .is_autocommit()
            .then(|| self.write_transaction())
            .transpose()?;
        self.ensure_codex_evidence_scope(settings, source)?;
        self.record_codex_identity(source, identity, stamp, "observed", None)?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(())
    }

    pub(crate) fn record_codex_switch(
        &self,
        settings: &Settings,
        source: &Source,
        result: &Value,
    ) -> Result<()> {
        let Some(start) = result["switchStartedAt"].as_i64() else {
            return Ok(());
        };
        let end = result["switchedAt"].as_i64().unwrap_or(start);
        let tx = self
            .db
            .is_autocommit()
            .then(|| self.write_transaction())
            .transpose()?;
        self.ensure_codex_evidence_scope(settings, source)?;
        self.record_codex_identity(
            source,
            result["previousIdentity"].as_str().unwrap_or(""),
            start,
            "switchBefore",
            None,
        )?;
        self.record_codex_identity(
            source,
            result["currentIdentity"].as_str().unwrap_or(""),
            end,
            "switch",
            Some(start),
        )?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(())
    }

    /// A home/host rebind must never inherit the previous location's evidence.
    pub(crate) fn ensure_codex_evidence_scope(
        &self,
        settings: &Settings,
        source: &Source,
    ) -> Result<()> {
        if source.provider != "codex" {
            return Ok(());
        }
        let tx = self
            .db
            .is_autocommit()
            .then(|| self.write_transaction())
            .transpose()?;
        let scope = evidence_scope(settings, source);
        let current = self.settings()?;
        let current_source = current
            .sources
            .iter()
            .find(|s| s.id == source.id)
            .context("目录已移除，已丢弃旧身份观测")?;
        ensure!(
            evidence_scope(&current, current_source) == scope,
            "目录或机器配置已变化，已丢弃旧身份观测"
        );
        let key = format!("codex-evidence-scope:{}", source.id);
        let old: Option<String> = self
            .db
            .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
            .optional()?;
        if old.as_deref() == Some(&scope) {
            if let Some(tx) = tx {
                tx.commit()?;
            }
            return Ok(());
        }
        for table in [
            "codex_session_creators",
            "codex_account_timeline",
            "codex_request_limits",
        ] {
            self.db.execute(
                &format!("DELETE FROM {table} WHERE source_id=?1"),
                [&source.id],
            )?;
        }
        self.db.execute(
            "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, scope],
        )?;
        if let Some(tx) = tx {
            tx.commit()?;
        }
        Ok(())
    }

    /// Returns an identity key, not an account label on the usage event.
    pub(crate) fn codex_usage_identity(
        &self,
        event: &UsageEvent,
        source_ids: &[String],
    ) -> Result<std::result::Result<String, &'static str>> {
        let Some(start) = event.interval_start else {
            return Ok(Err("缺少请求起始时间"));
        };
        let memberships: Vec<String> = self.db.prepare("SELECT source_id FROM event_sources WHERE event_id=?1 AND source_id IN (SELECT value FROM json_each(?2))")?.query_map(params![event.id,serde_json::to_string(source_ids)?], |r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut accepted = None;
        for source in memberships {
            let creator: Option<(String,i64,bool)> = self.db.query_row("SELECT identity_key,created_at,conflict FROM codex_session_creators WHERE source_id=?1 AND session_id=?2", params![source,event.session_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if creator.as_ref().is_some_and(|(_, _, conflict)| *conflict) {
                return Ok(Err("会话创建者身份冲突"));
            }
            let before: Option<(i64,String)> = self.db.query_row("SELECT stamp,identity_key FROM codex_account_timeline WHERE source_id=?1 AND stamp<=?2 ORDER BY stamp DESC,id DESC LIMIT 1",params![source,start],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let after: Option<(i64,String)> = self.db.query_row("SELECT stamp,identity_key FROM codex_account_timeline WHERE source_id=?1 AND stamp>=?2 ORDER BY stamp,id LIMIT 1",params![source,event.timestamp],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let (Some((left, identity)), Some((right, end_identity))) = (before, after) else {
                continue;
            };
            if identity.is_empty() || identity != end_identity || right - left > MAX_GAP {
                return Ok(Err("请求跨账户或身份观测存在缺口"));
            }
            let conflict: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM codex_account_timeline WHERE source_id=?1 AND ((stamp>?2 AND stamp<=?3 AND identity_key<>?4) OR (uncertain_from IS NOT NULL AND uncertain_from<?3 AND stamp>?2)))",params![source,start,event.timestamp,identity],|r|r.get(0))?;
            if conflict {
                return Ok(Err("请求跨越账户切换或不确定区间"));
            }
            // A resumed thread may keep A as creator while the current epoch uses B.
            // A newly created thread disagreeing with its epoch indicates a missed switch.
            if creator.as_ref().is_some_and(|(key, created, _)| {
                *created >= left && *created <= event.timestamp && key != &identity
            }) {
                return Ok(Err("新会话创建者与使用区间身份不一致"));
            }
            if accepted.as_ref().is_some_and(|old| old != &identity) {
                return Ok(Err("重复采集来源的账户证据不一致"));
            }
            accepted = Some(identity);
        }
        Ok(accepted.ok_or("账户时间线未覆盖请求区间"))
    }

    /// Request-side quota fingerprints can disprove an apparently stable login epoch.
    pub(crate) fn codex_request_quota_matches(
        &self,
        event: &UsageEvent,
        ids: &[String],
        account_key: &str,
        from: i64,
        to: i64,
    ) -> Result<bool> {
        let mut query = self.db.prepare("SELECT payload FROM codex_request_limits WHERE source_id IN (SELECT value FROM json_each(?1)) AND request_id=?2")?;
        let requests = query
            .query_map(
                params![
                    serde_json::to_string(ids)?,
                    event.import_identity.as_deref().unwrap_or(&event.id)
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if requests.is_empty() {
            let settings = self.settings()?;
            let expected = settings
                .accounts
                .iter()
                .find(|a| crate::accounts::key(a) == account_key)
                .and_then(|a| a.identity_key.as_deref());
            let mut creators = self.db.prepare("SELECT c.source_id,c.identity_key FROM codex_session_creators c JOIN event_sources s ON s.source_id=c.source_id AND s.event_id=?1 WHERE c.session_id=?2 AND c.source_id IN (SELECT value FROM json_each(?3))")?;
            for row in creators.query_map(
                params![event.id, event.session_id, serde_json::to_string(ids)?],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )? {
                let (source, creator) = row?;
                if expected == Some(creator.as_str()) {
                    continue;
                }
                // A polled credential change does not prove that an old process
                // restarted. Without request quota evidence, only our explicit
                // close-and-switch boundary can override the original creator.
                let transition: Option<(String,String)> = self.db.query_row("SELECT identity_key,kind FROM codex_account_timeline WHERE source_id=?1 AND stamp<=?2 AND (kind='switch' OR uncertain_from IS NOT NULL) ORDER BY stamp DESC,id DESC LIMIT 1",params![source,event.interval_start.unwrap_or(0)],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if !transition.is_some_and(|(identity, kind)| {
                    kind == "switch" && Some(identity.as_str()) == expected
                }) {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let mut snapshots = self.db.prepare("SELECT payload FROM quota_history WHERE account_key=?1 AND stamp>=?2 AND stamp<=?3 ORDER BY stamp")?;
        let snapshots = snapshots
            .query_map(params![account_key, from - 120, to + 120], |r| {
                r.get::<_, String>(0)
            })?
            .map(|r| Ok(serde_json::from_str::<QuotaSnapshot>(&r?)?))
            .collect::<Result<Vec<_>>>()?;
        if snapshots.is_empty() {
            return Ok(false);
        }
        for raw in requests {
            let request: QuotaSnapshot = serde_json::from_str(&raw)?;
            let compatible = request.windows.iter().any(|w| w.window_minutes.is_some())
                && snapshots.iter().any(|q| {
                    q.error.is_none()
                        && q.plan == request.plan
                        && request
                            .windows
                            .iter()
                            .filter(|w| w.window_minutes.is_some())
                            .all(|w| {
                                q.windows.iter().any(|observed| {
                                    observed.window_minutes == w.window_minutes
                                        && observed
                                            .resets_at
                                            .zip(w.resets_at)
                                            .is_some_and(|(a, b)| a.abs_diff(b) <= 10)
                                })
                            })
                });
            if !compatible {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store, Source, UsageEvent) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let source = Source {
            id: "s".into(),
            name: "Shared".into(),
            provider: "codex".into(),
            path: dir.path().to_string_lossy().into(),
            codex_home_id: Some("home".into()),
            ..Default::default()
        };
        let settings = Settings {
            sources: vec![source.clone()],
            accounts: ["a", "b"]
                .into_iter()
                .map(|id| Account {
                    id: id.into(),
                    name: id.into(),
                    provider: "codex".into(),
                    identity_key: Some(hash(id)),
                    quota_profile_id: Some(format!("home:{id}")),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        store
            .ensure_codex_evidence_scope(&settings, &source)
            .unwrap();
        let event = UsageEvent {
            id: "request".into(),
            turn_id: "turn".into(),
            import_identity: None,
            source_id: "s".into(),
            account_id: String::new(),
            provider: "codex".into(),
            session_id: "thread".into(),
            model: "model".into(),
            timestamp: 1015,
            tokens: Tokens {
                input: 100,
                ..Default::default()
            },
            attribution: "event".into(),
            billing: BillingEvidence {
                category: "subscription".into(),
                evidence: "fixture".into(),
            },
            native_credits: None,
            raw_tokens: None,
            interval_start: Some(1010),
            interval_evidence: "task-request-v2".into(),
        };
        store.put_event(&event, &[], &settings).unwrap();
        store
            .db
            .execute(
                "INSERT INTO codex_session_creators VALUES('s','thread',?1,1005,0)",
                [hash("a")],
            )
            .unwrap();
        (dir, store, source, event)
    }
    #[test]
    fn resumed_thread_follows_switch_epoch_instead_of_original_creator() {
        let (_dir, s, source, mut e) = fixture();
        s.record_codex_identity(&source, &hash("a"), 1000, "observed", None)
            .unwrap();
        s.record_codex_identity(&source, &hash("a"), 1030, "observed", None)
            .unwrap();
        assert_eq!(
            s.codex_usage_identity(&e, &["s".into()]).unwrap(),
            Ok(hash("a"))
        );
        s.record_codex_switch(&s.settings().unwrap(), &source,&json!({"switchStartedAt":1040,"switchedAt":1041,"previousIdentity":hash("a"),"currentIdentity":hash("b")})).unwrap();
        s.record_codex_identity(&source, &hash("b"), 1070, "observed", None)
            .unwrap();
        e.interval_start = Some(1042);
        e.timestamp = 1045;
        assert_eq!(
            s.codex_usage_identity(&e, &["s".into()]).unwrap(),
            Ok(hash("b"))
        );
        assert!(
            s.codex_request_quota_matches(&e, &["s".into()], "codex:b", 1041, 1070)
                .unwrap(),
            "a controlled process restart can override the thread creator"
        );
        let label: String =
            s.db.query_row(
                "SELECT account_id FROM events WHERE id='request'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            label.is_empty(),
            "valuation must not relabel shared history"
        );
    }
    #[test]
    fn round_trip_switch_and_external_uncertainty_do_not_pair_identical_endpoints() {
        let (_dir, s, source, mut e) = fixture();
        s.record_codex_identity(&source, &hash("a"), 1000, "observed", None)
            .unwrap();
        s.record_codex_switch(&s.settings().unwrap(), &source,&json!({"switchStartedAt":1020,"switchedAt":1021,"previousIdentity":hash("a"),"currentIdentity":hash("b")})).unwrap();
        s.record_codex_switch(&s.settings().unwrap(), &source,&json!({"switchStartedAt":1040,"switchedAt":1041,"previousIdentity":hash("b"),"currentIdentity":hash("a")})).unwrap();
        s.record_codex_identity(&source, &hash("a"), 1070, "observed", None)
            .unwrap();
        e.interval_start = Some(1010);
        e.timestamp = 1060;
        assert!(s.codex_usage_identity(&e, &["s".into()]).unwrap().is_err());
        s.record_codex_identity(&source, &hash("b"), 1100, "observed", None)
            .unwrap();
        e.interval_start = Some(1075);
        e.timestamp = 1085;
        assert!(s.codex_usage_identity(&e, &["s".into()]).unwrap().is_err());
        e.interval_start = Some(1101);
        e.timestamp = 1105;
        assert!(
            !s.codex_request_quota_matches(&e, &["s".into()], "codex:b", 1100, 1130)
                .unwrap(),
            "polling alone cannot prove that an old process changed accounts"
        );
    }
    #[test]
    fn creator_conflict_long_gap_and_location_rebind_reject_evidence() {
        let (_dir, s, mut source, e) = fixture();
        s.record_codex_identity(&source, &hash("a"), 1000, "observed", None)
            .unwrap();
        s.record_codex_identity(&source, &hash("a"), 1030, "observed", None)
            .unwrap();
        s.db.execute(
            "UPDATE codex_session_creators SET identity_key=?1",
            [hash("b")],
        )
        .unwrap();
        assert!(s.codex_usage_identity(&e, &["s".into()]).unwrap().is_err());
        s.db.execute("DELETE FROM codex_session_creators", [])
            .unwrap();
        s.db.execute("DELETE FROM codex_account_timeline WHERE stamp=1030", [])
            .unwrap();
        s.record_codex_identity(&source, &hash("a"), 1400, "observed", None)
            .unwrap();
        assert!(s.codex_usage_identity(&e, &["s".into()]).unwrap().is_err());
        source.path.push_str("/another-home");
        let old_settings = s.settings().unwrap();
        let mut changed = old_settings.clone();
        changed.sources[0] = source.clone();
        // Simulate a rebind commit. Ordinary settings intentionally refuse a
        // managed-home move; this test exercises a late response after persistence.
        s.db.execute(
            "UPDATE kv SET value=?1 WHERE key='settings'",
            [serde_json::to_string(&changed).unwrap()],
        )
        .unwrap();
        s.ensure_codex_evidence_scope(&changed, &source).unwrap();
        let count: i64 =
            s.db.query_row("SELECT COUNT(*) FROM codex_account_timeline", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        s.record_codex_identity(&source, &hash("b"), 1500, "observed", None)
            .unwrap();
        assert!(
            s.record_codex_observation(&old_settings, &old_settings.sources[0], &hash("a"), 1600)
                .is_err()
        );
        let key: String =
            s.db.query_row("SELECT identity_key FROM codex_account_timeline", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(key, hash("b"));
    }
    #[test]
    fn request_quota_signature_must_match_the_candidate_account() {
        let (_dir, s, _source, e) = fixture();
        let q = QuotaSnapshot {
            provider: "codex".into(),
            account_id: "a".into(),
            source_id: "s".into(),
            origin: "live".into(),
            plan: Some("plus".into()),
            updated_at: 1010,
            windows: vec![QuotaWindow {
                id: "five".into(),
                window_minutes: Some(300),
                resets_at: Some(10000),
                ..Default::default()
            }],
            ..Default::default()
        };
        s.quota(&q).unwrap();
        let mut request = q.clone();
        request.account_id.clear();
        request.windows[0].resets_at = Some(20000);
        s.db.execute(
            "INSERT INTO codex_request_limits VALUES('s','request',?1)",
            [serde_json::to_string(&request).unwrap()],
        )
        .unwrap();
        assert!(
            !s.codex_request_quota_matches(&e, &["s".into()], "codex:a", 1000, 1030)
                .unwrap()
        );
        request.windows[0].resets_at = Some(10005);
        s.db.execute(
            "UPDATE codex_request_limits SET payload=?1",
            [serde_json::to_string(&request).unwrap()],
        )
        .unwrap();
        assert!(
            s.codex_request_quota_matches(&e, &["s".into()], "codex:a", 1000, 1030)
                .unwrap()
        );
        request.windows[0].window_minutes = None;
        s.db.execute(
            "UPDATE codex_request_limits SET payload=?1",
            [serde_json::to_string(&request).unwrap()],
        )
        .unwrap();
        assert!(
            !s.codex_request_quota_matches(&e, &["s".into()], "codex:a", 1000, 1030)
                .unwrap(),
            "a quota without a comparable window cannot confirm an account"
        );
    }
    #[test]
    fn metadata_is_hashed_shared_and_replay_conflicts_are_retained() {
        let (_dir, s, source, e) = fixture();
        s.db.execute("DELETE FROM codex_session_creators", [])
            .unwrap();
        let state = ParseState {
            session_id: e.session_id.clone(),
            ..Default::default()
        };
        let mut row = json!({"type":"session_meta","payload":{"id":"thread","timestamp":"2026-10-20T00:00:00Z","creator_user_id":"PRIVATE_USER","creator_account_id":"PRIVATE_WORKSPACE"}});
        s.record_codex_history(&source, &state, &row, None).unwrap();
        s.record_codex_history(&source, &state, &row, None).unwrap();
        let (key, conflict): (String, bool) =
            s.db.query_row(
                "SELECT identity_key,conflict FROM codex_session_creators",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(key, hash("PRIVATE_USER\0PRIVATE_WORKSPACE"));
        assert!(!conflict);
        assert!(!key.contains("PRIVATE"));
        row["payload"]["creator_user_id"] = json!("another");
        s.record_codex_history(&source, &state, &row, None).unwrap();
        assert!(
            s.db.query_row("SELECT conflict FROM codex_session_creators", [], |r| r
                .get::<_, bool>(0))
                .unwrap()
        );
    }
    #[test]
    fn public_dashboard_ignores_legacy_account_usage_filters() {
        let (dir, s, _source, mut event) = fixture();
        event.id = "today-request".into();
        event.timestamp = now();
        event.interval_start = Some(now() - 5);
        s.put_event(&event, &[], &s.settings().unwrap()).unwrap();
        let mut engine = crate::Engine::open(dir.path()).unwrap();
        for account in ["a", "b", ""] {
            let d = engine
                .call("dashboard", json!({"provider":"codex","accountId":account}))
                .unwrap();
            assert_eq!(d["summary"]["total"], 100);
        }
    }
}
