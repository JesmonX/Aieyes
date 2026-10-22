//! Reversible native-history projection. Only metering metadata is retained.
use crate::{models::*, store::Store};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::json;

pub(crate) fn setup(store: &Store) -> Result<()> {
    store.db.execute_batch("CREATE TABLE IF NOT EXISTS usage_inheritance(
        event_id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,origin TEXT NOT NULL,inherited INTEGER NOT NULL,
        payload TEXT NOT NULL,price TEXT);
        CREATE INDEX IF NOT EXISTS usage_inheritance_origin ON usage_inheritance(scope,origin);
        CREATE TRIGGER IF NOT EXISTS usage_inheritance_delete AFTER DELETE ON events BEGIN DELETE FROM usage_inheritance WHERE event_id=OLD.id; END;")?;
    let tx = store.write_transaction()?;
    store.db.execute(
        "INSERT OR IGNORE INTO kv VALUES('counting.generation','1')",
        [],
    )?;
    let migrated: bool = store.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM kv WHERE key='counting.global-default.v1')",
        [],
        |r| r.get(0),
    )?;
    if !migrated {
        // Upgrade exactly once, including users whose old default was persisted as false.
        // Do not deserialize/re-save unrelated settings during this migration.
        store.db.execute("UPDATE kv SET value=json_remove(json_set(value,'$.preciseCounting',json('true')),'$.codexPreciseCounting') WHERE key='settings'", [])?;
        store.db.execute(
            "INSERT INTO kv VALUES('counting.global-default.v1','true')",
            [],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn generation(store: &Store) -> Result<u64> {
    Ok(store.db.query_row(
        "SELECT CAST(value AS INTEGER) FROM kv WHERE key='counting.generation'",
        [],
        |r| r.get(0),
    )?)
}

pub(crate) fn observe(
    store: &Store,
    event: &mut UsageEvent,
    price: Option<&ModelPrice>,
    settings: &Settings,
) -> Result<()> {
    let Some(proof) = &event.inheritance else {
        return Ok(());
    };
    let origin = proof.origin.clone();
    let scope = crate::usage::event_id(
        &event.provider,
        &event.source_id,
        &event.account_id,
        "native-inheritance",
    );
    store.db.execute("INSERT INTO usage_inheritance(event_id,scope,origin,inherited,payload,price) VALUES(?1,?2,?3,?4,?5,?6)
        ON CONFLICT(event_id) DO UPDATE SET scope=excluded.scope,origin=excluded.origin,inherited=excluded.inherited,payload=excluded.payload,price=excluded.price
        WHERE usage_inheritance.scope IS NOT excluded.scope OR usage_inheritance.origin IS NOT excluded.origin OR usage_inheritance.inherited IS NOT excluded.inherited OR usage_inheritance.payload IS NOT excluded.payload OR usage_inheritance.price IS NOT excluded.price",
        params![event.id,scope,proof.origin,proof.inherited,serde_json::to_string(event)?,price.map(serde_json::to_string).transpose()?])?;
    project(
        store,
        &scope,
        &origin,
        settings.precise_counting,
        Some(event),
    )
}

fn project(
    store: &Store,
    scope: &str,
    origin: &str,
    precise: bool,
    mut incoming: Option<&mut UsageEvent>,
) -> Result<()> {
    // The first observed non-inherited request owns the charge, independent of scan order
    // on later runs. Explicit seed records never manufacture an original request.
    let rows = store.db.prepare("SELECT i.event_id,i.inherited,i.payload,COALESCE(e.price,i.price),e.payload FROM usage_inheritance i LEFT JOIN events e ON e.id=i.event_id WHERE i.scope=?1 AND i.origin=?2 ORDER BY i.rowid")?
        .query_map(params![scope,origin], |r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let owner = rows.iter().find(|r| !r.1).map(|r| r.0.as_str());
    for (id, inherited, raw, price, current) in &rows {
        let mut event: UsageEvent = serde_json::from_str(raw)?;
        if precise && (*inherited || Some(id.as_str()) != owner) {
            event.tokens = Tokens::default();
            event.native_credits = None;
            event.attribution = "inherited-history".into();
        } else {
            crate::native_credits::resolve(store, &mut event)?;
        }
        let payload = serde_json::to_string(&event)?;
        let is_incoming = incoming.as_ref().is_some_and(|next| next.id == *id);
        if is_incoming {
            **incoming.as_mut().unwrap() = event.clone();
        }
        if current.as_deref() == Some(&payload) {
            continue;
        }
        let frozen: Option<ModelPrice> = price.as_deref().map(serde_json::from_str).transpose()?;
        let (cost, covered) = crate::pricing::estimate(&event.tokens, frozen.as_ref());
        if !is_incoming {
            store.db.execute(
                "UPDATE events SET payload=?2,cost=?3,priced_tokens=?4 WHERE id=?1",
                params![id, payload, cost, covered],
            )?;
            crate::native_credits::persisted(store, &event)?;
        }
        let prior: Option<UsageEvent> = current.as_deref().map(serde_json::from_str).transpose()?;
        if prior.is_some_and(|prior| {
            prior.tokens != event.tokens || prior.native_credits != event.native_credits
        }) {
            store.db.execute(
                "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![
                    format!("history-correction-pending:{id}"),
                    json!({"eventId":id,"sourceId":event.source_id,"timestamp":event.timestamp})
                        .to_string()
                ],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn prepare_source(store: &Store, settings: &Settings, source: &Source) -> Result<()> {
    let key = format!("counting.source:{}", source.id);
    let revision = format!(
        "{}:{}",
        settings.precise_counting, settings.counting_generation
    );
    let saved: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [&key], |r| r.get(0))
        .optional()?;
    if saved.as_deref() == Some(&revision) {
        return Ok(());
    }
    store.prepare_history_corrections()?;
    let tx = store.write_transaction()?;
    crate::import::current_settings(store, settings, source)?;
    let groups = store.db.prepare("SELECT DISTINCT i.scope,i.origin FROM usage_inheritance i JOIN event_sources s ON s.event_id=i.event_id WHERE s.source_id=?1")?
        .query_map([&source.id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (scope, origin) in groups {
        project(store, &scope, &origin, settings.precise_counting, None)?;
    }
    store.db.execute(
        "INSERT INTO kv VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, revision],
    )?;
    tx.commit()?;
    store.repair_history_estimates()?;
    Ok(())
}

/// Pi's former account-wide entry identity must become a per-session observation.
/// Move the old row with its frozen price and every source association intact.
pub(crate) fn migrate_pi_identity(
    store: &Store,
    event: &UsageEvent,
    origin: &str,
    settings: &Settings,
) -> Result<()> {
    let old_id =
        crate::usage::event_id(&event.provider, &event.source_id, &event.account_id, origin);
    let prior: Option<(String, Option<String>)> = store
        .db
        .query_row(
            "SELECT payload,price FROM events WHERE id=?1",
            [&old_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((raw, price)) = prior else {
        return Ok(());
    };
    let mut old: UsageEvent = serde_json::from_str(&raw)?;
    old.import_identity = Some(format!("{}:{origin}", old.session_id));
    old.id = crate::usage::event_id(
        &old.provider,
        &old.source_id,
        &old.account_id,
        old.import_identity.as_deref().unwrap(),
    );
    old.inheritance = Some(UsageInheritance {
        origin: origin.into(),
        inherited: false,
    });
    let frozen: Option<ModelPrice> = price.as_deref().map(serde_json::from_str).transpose()?;
    store.merge_usage_event(old.clone(), &old.source_id, frozen.as_slice(), settings)?;
    store.db.execute("INSERT OR IGNORE INTO event_sources SELECT ?1,source_id FROM event_sources WHERE event_id=?2",params![old.id,old_id])?;
    store
        .db
        .execute("DELETE FROM event_sources WHERE event_id=?1", [&old_id])?;
    store
        .db
        .execute("DELETE FROM events WHERE id=?1", [old_id])?;
    Ok(())
}
