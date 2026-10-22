//! Durable counter corrections keep replay idempotent and retain the original audit data.
use crate::{codex_history::InheritedHistory, models::*, store::Store};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::json;

pub(crate) fn setup(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS usage_corrections(
        event_id TEXT PRIMARY KEY,payload TEXT NOT NULL,proof TEXT NOT NULL,
        original_payload TEXT,original_price TEXT,parser_version INTEGER NOT NULL,corrected_at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS estimate_correction_audit(kind TEXT NOT NULL,id TEXT NOT NULL,repair_id TEXT NOT NULL,original_payload TEXT NOT NULL,corrected_at INTEGER NOT NULL,PRIMARY KEY(kind,id,repair_id));")?;
    Ok(())
}

impl Store {
    pub(crate) fn repair_history_estimates(&self) -> Result<()> {
        use crate::estimates::Estimate;
        let pending: Vec<(String, String)> = self
            .db
            .prepare("SELECT key,value FROM kv WHERE key LIKE 'history-correction-pending:%'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if pending.is_empty() {
            return Ok(());
        }
        let tx = self.write_transaction()?;
        let records = self.estimates()?;
        let replaced: std::collections::HashSet<_> = records
            .iter()
            .filter_map(|e| e.original_estimate_id.clone())
            .collect();
        for original in records.iter().filter(|e| !replaced.contains(&e.id)) {
            let mut affected = false;
            for (_, raw) in &pending {
                let row: serde_json::Value = serde_json::from_str(raw)?;
                let stamp = row["timestamp"].as_i64().unwrap_or(0);
                if stamp <= original.started_at || stamp > original.checkpoint_at {
                    continue;
                }
                let belongs: bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM event_sources WHERE event_id=?1 AND source_id IN (SELECT value FROM json_each(?2)))",
                    rusqlite::params![row["eventId"].as_str().unwrap_or(""),serde_json::to_string(&original.source_ids)?],|r|r.get(0))?;
                affected |= belongs;
            }
            if !affected {
                continue;
            }
            let mut repaired = original.clone();
            let revision: String =
                self.db
                    .query_row("SELECT lower(hex(randomblob(8)))", [], |r| r.get(0))?;
            repaired.id = format!("{}-fork-v6-{revision}", original.id);
            repaired.original_estimate_id = Some(original.id.clone());
            repaired.repaired_at = Some(now());
            repaired.calculation_version = if repaired.sampling_mode == "automatic"
                && repaired.account_key.starts_with("codex:")
            {
                4
            } else {
                3
            };
            repaired.clear_values();
            if self.recalculate_history_values(&mut repaired).is_err() {
                repaired.clear_values();
                repaired.calculation_status = "historyUnresolved".into();
                repaired.calculation_note =
                    "历史修正后缺少原始区间或价格证据，此记录不再作为有效估值".into();
            }
            self.db.execute(
                "INSERT OR IGNORE INTO estimate_correction_audit VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    original.storage_kind(),
                    original.id,
                    repaired.id,
                    serde_json::to_string(original)?,
                    now()
                ],
            )?;
            if original.status != "completed" {
                let mut sealed = original.clone();
                sealed.status = "completed".into();
                sealed.ended_at = Some(sealed.checkpoint_at);
                self.save_estimate(&sealed)?;
            }
            // The replacement is canonical even when it has no value, so the old overestimate
            // cannot silently become the latest valid result again.
            if let Estimate::Quota(v) = &mut repaired
                && !v.consumed_percent.is_finite()
            {
                v.consumed_percent = 0.0;
            }
            self.save_estimate(&repaired)?;
        }
        for (key, _) in pending {
            self.db.execute("DELETE FROM kv WHERE key=?1", [key])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn prepare_history_corrections(&self) -> Result<()> {
        let key = "history-correction-backup-shared-v1";
        let done: bool =
            self.db
                .query_row("SELECT EXISTS(SELECT 1 FROM kv WHERE key=?1)", [key], |r| {
                    r.get(0)
                })?;
        let has_history: bool =
            self.db
                .query_row("SELECT EXISTS(SELECT 1 FROM events)", [], |r| r.get(0))?;
        if done || !has_history {
            return Ok(());
        }
        ensure!(self.db.is_autocommit(), "历史修复备份必须在导入事务前完成");
        let root = std::path::Path::new(self.db.path().unwrap())
            .parent()
            .unwrap();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("history-correction-backup.lock"))?;
        let started = std::time::Instant::now();
        let _guard = loop {
            match crate::file_lock::FileLock::try_exclusive(file.try_clone()?) {
                Ok(guard) => break guard,
                Err(std::fs::TryLockError::WouldBlock) if started.elapsed().as_secs() < 30 => {
                    crate::operations::check()?;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(error) => return Err(error.into()),
            }
        };
        if self
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM kv WHERE key=?1)", [key], |r| {
                r.get::<_, bool>(0)
            })?
        {
            return Ok(());
        }
        let folder = root.join("history-backups");
        std::fs::create_dir_all(&folder)?;
        let backup = folder.join(format!(
            "shared-counting-v1-{}-{}.sqlite",
            now(),
            std::process::id()
        ));
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(&backup)?);
        self.db
            .execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        self.db.execute(
            "INSERT OR IGNORE INTO kv VALUES(?1,?2)",
            params![key, backup.to_string_lossy()],
        )?;
        Ok(())
    }

    pub(crate) fn corrected_usage(&self, id: &str) -> Result<Option<UsageEvent>> {
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT payload FROM usage_corrections WHERE event_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn save_counter_correction(
        &self,
        event: &UsageEvent,
        proof: &InheritedHistory,
    ) -> Result<()> {
        let old: Option<(String, Option<String>)> = self
            .db
            .query_row(
                "SELECT payload,price FROM events WHERE id=?1",
                [&event.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        self.db.execute("INSERT INTO usage_corrections VALUES(?1,?2,?3,?4,?5,?6,?7)
            ON CONFLICT(event_id) DO UPDATE SET payload=excluded.payload,proof=excluded.proof,parser_version=excluded.parser_version,corrected_at=excluded.corrected_at
            WHERE usage_corrections.payload<>excluded.payload OR usage_corrections.proof<>excluded.proof",
            params![event.id,serde_json::to_string(event)?,serde_json::to_string(proof)?,old.as_ref().map(|v| &v.0),old.as_ref().and_then(|v|v.1.as_ref()),crate::usage::PARSER_VERSION,now()])?;
        if let Some((raw, _)) = old {
            let previous: UsageEvent = serde_json::from_str(&raw)?;
            if previous.tokens != event.tokens || previous.attribution != event.attribution {
                // Repair runs after the whole source replay, with all corrected events present.
                self.db.execute("INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                    params![format!("history-correction-pending:{}",event.id),json!({"eventId":event.id,"sourceId":event.source_id,"timestamp":event.timestamp}).to_string()])?;
            }
        }
        Ok(())
    }
}
