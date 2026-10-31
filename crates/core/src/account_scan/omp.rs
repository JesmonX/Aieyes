//! OMP 18.12.2 keeps its OAuth pool in `agent.db`, alongside unrelated state.
//! There is no single current row: enabled credentials are those without a
//! `disabled_cause`. Read only the supported upstreams, without selecting,
//! refreshing, deleting duplicates or writing any native state.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

pub(super) fn credentials(db: &rusqlite::Connection) -> Result<Value> {
    let mut result = serde_json::Map::new();
    if !crate::opencode::table_exists(db, "auth_credentials")? {
        return Ok(Value::Object(result));
    }
    // Filtering in SQL avoids fetching unrelated providers or disabled secrets.
    // Bound individual TEXT values before they are copied out of SQLite.
    let mut statement = db.prepare(
        "SELECT id,provider,CASE WHEN length(data)<=?1 THEN data END
         FROM auth_credentials
         WHERE disabled_cause IS NULL AND credential_type='oauth' AND id>0
           AND provider IN ('anthropic','openai','openai-codex','github-copilot','google-gemini-cli')
         ORDER BY id ASC LIMIT 257",
    )?;
    let rows = statement.query_map([super::LIMIT], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut size = 0;
    for (index, row) in rows.enumerate() {
        ensure!(index < 256, "原生配置超过扫描大小限制");
        let (id, provider, raw) = row?;
        let raw = raw.context("原生配置超过扫描大小限制")?;
        size += raw.len() as u64;
        ensure!(size <= super::LIMIT, "原生配置超过扫描大小限制");
        // Like OMP's deserializeCredential, a malformed row is unavailable;
        // it must not prevent reading another valid member of the pool.
        let Ok(Value::Object(value)) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let mut credential = json!({"type":"oauth"});
        for field in [
            "access",
            "expires",
            "projectId",
            "email",
            "accountId",
            "enterpriseUrl",
        ] {
            if let Some(value) = value.get(field) {
                credential[field] = value.clone();
            }
        }
        // Copilot stores the durable GitHub token under `refresh`. For every
        // other upstream the refresh grant is unnecessary and stays unread by
        // callers of this projection, even during initial account discovery.
        if provider == "github-copilot"
            && let Some(refresh) = value.get("refresh")
        {
            credential["refresh"] = refresh.clone();
        }
        if let Some(metadata) = value.get("metadata").and_then(Value::as_object) {
            let fields = ["accountID", "accountId", "enterpriseUrl"]
                .into_iter()
                .filter_map(|field| {
                    metadata
                        .get(field)
                        .map(|value| (field.into(), value.clone()))
                })
                .collect();
            credential["metadata"] = Value::Object(fields);
        }
        // This private document is consumed exactly like legacy auth.json.
        // Its JSON pointer keeps the database row id stable across token changes.
        result.insert(format!("{provider}#{id}"), credential);
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SCHEMA: &str = include_str!("../../tests/fixtures/omp-auth.sql");

    #[test]
    fn omp_projects_only_enabled_supported_oauth_rows_and_needed_fields() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let value = credentials(&db).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "anthropic#8",
                "github-copilot#3",
                "google-gemini-cli#1",
                "google-gemini-cli#7",
                "openai-codex#9"
            ]
        );
        assert_eq!(value["google-gemini-cli#1"]["access"], "PRIVATE-GEMINI-ONE");
        assert_eq!(value["google-gemini-cli#1"]["projectId"], "project-one");
        assert_eq!(value["google-gemini-cli#1"]["expires"], 4102444800000_i64);
        assert_eq!(value["github-copilot#3"]["refresh"], "gho_PRIVATE-GITHUB");
        assert_eq!(
            value["openai-codex#9"]["metadata"],
            json!({"accountID":"workspace-one"})
        );
        let serialized = value.to_string();
        for secret in [
            "PRIVATE-GEMINI-DISABLED",
            "PRIVATE-DO-NOT-PARSE",
            "PRIVATE-ANTIGRAVITY",
            "PRIVATE-GEMINI-API-KEY",
            "PRIVATE-UNRELATED-FIELD",
            "PRIVATE-METADATA",
            "PRIVATE-GEMINI-REFRESH",
            "PRIVATE-CLAUDE-REFRESH",
            "PRIVATE-CODEX-REFRESH",
        ] {
            assert!(!serialized.contains(secret), "excluded field was projected");
        }
    }

    fn read(path: &Path) -> Value {
        let db = crate::sqlite_ro::open_ro(path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA query_only", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        credentials(&db).unwrap()
    }

    #[test]
    fn omp_committed_wal_rows_and_disable_changes_are_read_without_native_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let wal = dir.path().join("agent.db-wal");
        let before = (std::fs::read(&path).unwrap(), std::fs::read(&wal).unwrap());
        let first = read(&path);
        assert!(first.get("google-gemini-cli#1").is_some());
        assert_eq!(
            before,
            (std::fs::read(&path).unwrap(), std::fs::read(&wal).unwrap())
        );

        db.execute(
            "INSERT INTO auth_credentials VALUES(10,'google-gemini-cli','oauth',?1,NULL,NULL,1,10)",
            [json!({"access":"PRIVATE-NEW-ROW","projectId":"new-project"}).to_string()],
        )
        .unwrap();
        assert_eq!(
            read(&path)["google-gemini-cli#10"]["access"],
            "PRIVATE-NEW-ROW"
        );
        db.execute(
            "UPDATE auth_credentials SET disabled_cause='signed out' WHERE id=1",
            [],
        )
        .unwrap();
        assert!(read(&path).get("google-gemini-cli#1").is_none());
        db.execute(
            "UPDATE auth_credentials SET data=?1 WHERE id=7",
            [json!({"access":"PRIVATE-REPLACED","projectId":"replacement-project"}).to_string()],
        )
        .unwrap();
        let replaced = read(&path);
        assert_eq!(
            replaced["google-gemini-cli#7"]["access"],
            "PRIVATE-REPLACED"
        );
        assert_eq!(
            replaced["google-gemini-cli#7"]["projectId"],
            "replacement-project"
        );
    }

    #[test]
    fn omp_overflow_numbers_skip_only_the_bad_pool_row() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db.execute(
            "INSERT INTO auth_credentials VALUES(90,'google-gemini-cli','oauth',?1,NULL,NULL,1,1)",
            [json!({"access":"PRIVATE-EXACT-INTEGER","expires":i64::MAX,"projectId":"project-exact"}).to_string()],
        ).unwrap();
        let expected = credentials(&db).unwrap();
        let mut bad_numbers = ["NaN", "Infinity", "-Infinity", "1e400", "-1e400", "1.9e308"]
            .map(str::to_owned)
            .to_vec();
        bad_numbers.extend(["9".repeat(400), format!("-{}", "9".repeat(400))]);
        for (index, number) in bad_numbers.iter().enumerate() {
            let raw = format!("{{\"access\":\"PRIVATE-BAD-NUMBER\",\"expires\":{number}}}");
            db.execute("INSERT INTO auth_credentials VALUES(?1,'google-gemini-cli','oauth',?2,NULL,NULL,1,1)", rusqlite::params![100 + index as i64, raw]).unwrap();
        }
        let actual = credentials(&db).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual["google-gemini-cli#90"]["expires"].as_i64(),
            Some(i64::MAX)
        );
        assert!(!actual.to_string().contains("PRIVATE-BAD-NUMBER"));
    }

    #[test]
    fn omp_bad_rows_are_unavailable_and_oversized_supported_data_is_bounded() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        assert_eq!(credentials(&db).unwrap(), json!({}));
        db.execute_batch(SCHEMA).unwrap();
        db.execute("INSERT INTO auth_credentials VALUES(10,'google-gemini-cli','oauth','{',NULL,NULL,1,10)", []).unwrap();
        db.execute("INSERT INTO auth_credentials VALUES(12,'google-gemini-cli','oauth','{\"access\":\"PRIVATE-MALFORMED\",\"expires\":NaN}',NULL,NULL,1,12)", []).unwrap();
        assert_eq!(credentials(&db).unwrap().as_object().unwrap().len(), 5);
        db.execute(
            "INSERT INTO auth_credentials VALUES(11,'unrelated','oauth',?1,NULL,NULL,1,11)",
            ["x".repeat(super::super::LIMIT as usize + 1)],
        )
        .unwrap();
        assert!(
            credentials(&db).is_ok(),
            "unrelated credential data is not fetched"
        );
        db.execute(
            "UPDATE auth_credentials SET provider='google-gemini-cli' WHERE id=11",
            [],
        )
        .unwrap();
        assert_eq!(
            credentials(&db).unwrap_err().to_string(),
            "原生配置超过扫描大小限制"
        );
    }
}
