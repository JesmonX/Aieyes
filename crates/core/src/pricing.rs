use crate::models::*;
use crate::store::Store;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{fs::File, path::Path, time::Duration};

const OPENROUTER_SYNC_INTERVAL: i64 = 12 * 60 * 60;
const OPENROUTER_RETRY_INTERVAL: i64 = 15 * 60;

impl Store {
    fn openrouter_sync_due(&self, at: i64) -> Result<bool> {
        let (synced, attempted): (Option<i64>, Option<i64>) = self.db.query_row(
            "SELECT
                (SELECT json_extract(value, '$[0].fetchedAt') FROM kv WHERE key='openrouter-models'),
                (SELECT CAST(value AS INTEGER) FROM kv WHERE key='openrouter-sync-attempt')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let elapsed = |stamp: Option<i64>, interval| {
            stamp.is_none_or(|stamp| at < stamp || at - stamp >= interval)
        };
        Ok(elapsed(synced, OPENROUTER_SYNC_INTERVAL)
            && elapsed(attempted, OPENROUTER_RETRY_INTERVAL))
    }

    pub(crate) fn sync_openrouter_prices(&self, automatic: bool) -> Result<Option<Value>> {
        if automatic && !self.openrouter_sync_due(now())? {
            return Ok(None);
        }
        let file = File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("openrouter-prices.lock"))?;
        let _lock = match crate::file_lock::FileLock::try_exclusive(file) {
            Ok(lock) => lock,
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        // Another process may have completed the sync while we checked its timestamp.
        if automatic {
            let at = now();
            if !self.openrouter_sync_due(at)? {
                return Ok(None);
            }
            let tx = self.write_transaction()?;
            tx.execute(
                "INSERT OR REPLACE INTO kv(key,value) VALUES('openrouter-sync-attempt',?1)",
                [at.to_string()],
            )?;
            tx.commit()?;
        }
        let response = crate::network::client(&self.settings()?.proxy)?
            .get("https://openrouter.ai/api/v1/models")
            .send()
            .map_err(|_| anyhow::anyhow!("价格连接失败"))?;
        ensure!(
            response.status().is_success(),
            "价格同步失败（HTTP {}）",
            response.status().as_u16()
        );
        let prices = parse_openrouter(&response.json::<Value>()?);
        ensure!(!prices.is_empty(), "价格列表为空");
        let repriced = self.save_openrouter_prices(&prices)?;
        Ok(Some(serde_json::json!({
            "models": prices.len(),
            "repriced": repriced,
            "updatedAt": now()
        })))
    }
}

/// Run on the desktop's background core connection, independently of view visibility.
pub fn spawn_openrouter_sync(root: &Path) {
    if std::env::var("AIEYES_AUTO_SCAN").is_ok_and(|v| v == "off") {
        return;
    }
    let root = root.to_owned();
    let _ = std::thread::Builder::new()
        .name("aieyes-openrouter-prices".into())
        .spawn(move || {
            let mut store = None;
            loop {
                if store.is_none() {
                    store = Store::open(&root).ok();
                }
                if let Some(store) = &store
                    && let Ok(_scope) = crate::operations::enter("prices.sync")
                    && let Err(error) = store.sync_openrouter_prices(true)
                {
                    eprintln!("Aieyes: OpenRouter 价格自动同步失败：{error:#}");
                }
                std::thread::sleep(Duration::from_secs(60));
            }
        });
}

pub fn parse_openrouter(v: &Value) -> Vec<ModelPrice> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let p = &m["pricing"];
            let price = |key: &str| {
                p[key]
                    .as_str()
                    .and_then(|s| s.parse::<f64>().ok())
                    .or(p[key].as_f64())
                    .filter(|v| v.is_finite() && *v >= 0.0)
            };
            Some(ModelPrice {
                id: m["id"].as_str()?.into(),
                name: m["name"].as_str().unwrap_or("").into(),
                input: price("prompt"),
                output: price("completion"),
                cache_read: price("input_cache_read"),
                cache_write: price("input_cache_write"),
                fetched_at: now(),
            })
        })
        .collect()
}

pub fn estimate(tokens: &Tokens, price: Option<&ModelPrice>) -> (f64, u64) {
    let Some(p) = price else { return (0.0, 0) };
    [
        (tokens.input, p.input),
        (tokens.output, p.output),
        (tokens.cache_read, p.cache_read),
        (tokens.cache_write, p.cache_write),
    ]
    .into_iter()
    .fold((0.0, 0), |(cost, covered), (n, rate)| match rate {
        Some(r) => (cost + n as f64 * r, covered + n),
        None => (cost, covered),
    })
}

pub fn missing_tokens(t: &Tokens, p: Option<&ModelPrice>) -> Tokens {
    Tokens {
        input: if p.and_then(|p| p.input).is_none() {
            t.input
        } else {
            0
        },
        output: if p.and_then(|p| p.output).is_none() {
            t.output
        } else {
            0
        },
        cache_read: if p.and_then(|p| p.cache_read).is_none() {
            t.cache_read
        } else {
            0
        },
        cache_write: if p.and_then(|p| p.cache_write).is_none() {
            t.cache_write
        } else {
            0
        },
        reasoning: 0,
    }
}

pub fn find_price<'a>(
    model: &str,
    prices: &'a [ModelPrice],
    mappings: &std::collections::BTreeMap<String, String>,
) -> Option<&'a ModelPrice> {
    find_usage_price("", model, prices, mappings)
}

pub(crate) fn effort_base(model: &str, agy: bool) -> Option<&str> {
    let name = model.rsplit('/').next()?;
    let (base, effort) = name.rsplit_once('-')?;
    let standard = matches!(effort, "minimal" | "low" | "medium" | "high");
    let supported = (standard && base.starts_with("gemini-"))
        || (agy && matches!(effort, "low" | "medium" | "high" | "xhigh" | "max"));
    (supported && !base.is_empty()).then(|| &model[..model.len() - effort.len() - 1])
}

pub(crate) fn find_usage_price<'a>(
    provider: &str,
    model: &str,
    prices: &'a [ModelPrice],
    mappings: &std::collections::BTreeMap<String, String>,
) -> Option<&'a ModelPrice> {
    if crate::antigravity::unknown_model(model) {
        return None;
    }
    exact_price(model, prices, mappings).or_else(|| {
        if mappings.contains_key(model) {
            return None;
        }
        let agy = crate::providers::antigravity(provider);
        let base = effort_base(model, agy)?;
        if let Some(price) = exact_price(base, prices, mappings) {
            return Some(price);
        }
        if !agy || mappings.contains_key(base) {
            return None;
        }
        // AGY can spell a decimal version with a hyphen (e.g. Opus 5-5).
        // Match identical slugs and namespaces, with only numeric dots normalized.
        let mut candidates = prices.iter().filter(|p| {
            let id = if base.contains('/') {
                &p.id
            } else if let Some((_, name)) = p.id.split_once('/') {
                name
            } else {
                &p.id
            };
            let bytes = id.as_bytes();
            !id.contains(':')
                && bytes
                    .iter()
                    .enumerate()
                    .map(|(i, &b)| {
                        if b == b'.'
                            && i > 0
                            && bytes[i - 1].is_ascii_digit()
                            && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                        {
                            b'-'
                        } else {
                            b
                        }
                    })
                    .eq(base.bytes())
        });
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first)
    })
}

fn exact_price<'a>(
    model: &str,
    prices: &'a [ModelPrice],
    mappings: &std::collections::BTreeMap<String, String>,
) -> Option<&'a ModelPrice> {
    if let Some(id) = mappings.get(model) {
        return prices.iter().find(|p| &p.id == id);
    }
    if let Some(p) = prices.iter().find(|p| p.id == model) {
        return Some(p);
    }
    // Only exact suffixes; never silently turn an unknown model into a different model.
    let matches: Vec<_> = prices
        .iter()
        .filter(|p| p.id.split_once('/').is_some_and(|(_, name)| name == model))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

#[derive(Debug)]
struct ModelIdentity {
    family: Vec<String>,
    version: Vec<String>,
    date: Option<String>,
    latest: bool,
    batch: bool,
}

fn official_owner(value: &str) -> Option<&'static str> {
    match value
        .to_ascii_lowercase()
        .replace([' ', '-', '_', '.'], "")
        .as_str()
    {
        "openai" => Some("openai"),
        "anthropic" => Some("anthropic"),
        "google" => Some("google"),
        "meta" | "metallama" => Some("meta-llama"),
        "xai" => Some("x-ai"),
        "deepseek" => Some("deepseek"),
        "qwen" | "alibaba" => Some("qwen"),
        "mistral" | "mistralai" => Some("mistralai"),
        "cohere" => Some("cohere"),
        "moonshotai" | "moonshot" => Some("moonshotai"),
        "zai" | "zhipu" => Some("z-ai"),
        "minimax" => Some("minimax"),
        "nvidia" => Some("nvidia"),
        "bytedance" | "bytedanceseed" => Some("bytedance-seed"),
        _ => None,
    }
}

fn model_identity(value: &str) -> ModelIdentity {
    let value = value.rsplit('/').next().unwrap_or(value);
    // Display names often start with the model author, e.g. "Anthropic: Claude …".
    let value = value
        .split_once(':')
        .filter(|(prefix, _)| official_owner(prefix).is_some())
        .map_or(value, |(_, name)| name);
    let mut tokens = Vec::<String>::new();
    let mut word = String::new();
    for c in value.to_lowercase().chars() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
        } else {
            if word
                .chars()
                .last()
                .is_some_and(|last| last.is_ascii_digit() != c.is_ascii_digit())
            {
                tokens.push(std::mem::take(&mut word));
            }
            word.push(c);
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    let mut identity = ModelIdentity {
        family: vec![],
        version: vec![],
        date: None,
        latest: false,
        batch: false,
    };
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if token.len() == 4
            && token.starts_with("20")
            && tokens
                .get(i + 1)
                .is_some_and(|v| v.len() == 2 && v.bytes().all(|b| b.is_ascii_digit()))
            && tokens
                .get(i + 2)
                .is_some_and(|v| v.len() == 2 && v.bytes().all(|b| b.is_ascii_digit()))
        {
            identity.date = Some(format!("{}{}{}", token, tokens[i + 1], tokens[i + 2]));
            i += 3;
            continue;
        }
        match token.as_str() {
            "latest" => identity.latest = true,
            "batch" => identity.batch = true,
            "v" if tokens
                .get(i + 1)
                .is_some_and(|v| v.bytes().all(|b| b.is_ascii_digit())) => {}
            _ if token.bytes().all(|b| b.is_ascii_digit()) => {
                if token.len() == 8 && token.starts_with("20") {
                    identity.date = Some(token.clone());
                } else {
                    identity.version.push(token.clone());
                }
            }
            _ => identity.family.push(token.clone()),
        }
        i += 1;
    }
    identity.family.sort();
    identity
}

fn inferred_owner(identity: &ModelIdentity) -> Option<&'static str> {
    let has = |s: &str| identity.family.iter().any(|v| v == s);
    if has("claude") {
        Some("anthropic")
    } else if has("gpt") || has("chatgpt") || (has("o") && !identity.version.is_empty()) {
        Some("openai")
    } else if has("gemini") || has("gemma") {
        Some("google")
    } else if has("llama") {
        Some("meta-llama")
    } else if has("grok") {
        Some("x-ai")
    } else if has("kimi") {
        Some("moonshotai")
    } else if has("glm") {
        Some("z-ai")
    } else if has("mistral") || has("mixtral") || has("codestral") || has("devstral") {
        Some("mistralai")
    } else if has("seed") {
        Some("bytedance-seed")
    } else {
        identity.family.iter().find_map(|word| official_owner(word))
    }
}

/// Explicit estimation helper. Ordinary billing keeps exact/manual mappings;
/// callers persist the chosen reference so users can inspect and replace it.
fn smart_reference<'a>(
    model: &str,
    name: &str,
    prices: &'a [ModelPrice],
) -> Option<&'a ModelPrice> {
    if crate::antigravity::unknown_model(model) {
        return None;
    }
    let id = model_identity(model);
    let label = model_identity(name);
    let owner = model
        .split_once('/')
        .and_then(|(owner, _)| official_owner(owner))
        .or_else(|| inferred_owner(&id))
        .or_else(|| inferred_owner(&label));
    let mut matches = Vec::new();
    for price in prices {
        if price.input.is_none() && price.output.is_none() {
            continue;
        }
        let candidate_id = model_identity(&price.id);
        let candidate_name = model_identity(&price.name);
        // Batch is a distinct tariff even when a display name omits the suffix.
        if (id.batch || label.batch) != (candidate_id.batch || candidate_name.batch) {
            continue;
        }
        let mut best = None;
        for (source, from_id) in [(&id, true), (&label, false)] {
            if source.family.is_empty() {
                continue;
            }
            for target in [&candidate_id, &candidate_name] {
                if source.family != target.family {
                    continue;
                }
                // An informative ID takes precedence over a less specific display name.
                let versions = if !id.version.is_empty() {
                    &id.version
                } else {
                    &label.version
                };
                let date = id.date.as_ref().or(label.date.as_ref());
                let target_versions = if !candidate_id.version.is_empty() {
                    &candidate_id.version
                } else {
                    &target.version
                };
                let target_date = candidate_id.date.as_ref().or(target.date.as_ref());
                if !versions.is_empty() && versions != target_versions {
                    continue;
                }
                if date.is_some() && date != target_date {
                    continue;
                }
                let mut score = if from_id { 100 } else { 80 };
                if model.eq_ignore_ascii_case(&price.id)
                    && model
                        .split_once('/')
                        .and_then(|(o, _)| official_owner(o))
                        .is_some()
                {
                    score += 1000;
                }
                if owner.is_some()
                    && price
                        .id
                        .split_once('/')
                        .and_then(|(o, _)| official_owner(o))
                        == owner
                {
                    score += 300;
                }
                if versions.is_empty() && date.is_none() {
                    if !target_versions.is_empty() && !candidate_id.latest && !candidate_name.latest
                    {
                        continue;
                    }
                    score += if candidate_id.latest || candidate_name.latest {
                        60
                    } else if target_versions.is_empty() && target_date.is_none() {
                        50
                    } else {
                        0
                    };
                } else if date.is_none() && target_date.is_none() {
                    score += 30;
                }
                best = Some(best.map_or(score, |old: i32| old.max(score)));
            }
        }
        if let Some(score) = best {
            matches.push((score, price));
        }
    }
    matches
        .sort_by(|(a_score, a), (b_score, b)| b_score.cmp(a_score).then_with(|| a.id.cmp(&b.id)));
    matches.first().map(|(_, price)| *price)
}

impl crate::Engine {
    pub(crate) fn reference_prices(&mut self, params: Value) -> anyhow::Result<Value> {
        use anyhow::{Context, ensure};
        use rusqlite::OptionalExtension;
        use serde_json::json;
        let tx = self.store.write_transaction()?;
        let mut settings = self.store.settings()?;
        let prices = self.store.prices()?;
        ensure!(!prices.is_empty(), "请先同步 OpenRouter 模型价格");
        // Sync stores the catalog separately from manually entered prices.
        let catalog: Vec<ModelPrice> = self
            .store
            .db
            .query_row(
                "SELECT value FROM kv WHERE key='openrouter-models'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_else(|| {
                prices
                    .iter()
                    .filter(|p| {
                        p.id.split_once('/')
                            .and_then(|(o, _)| official_owner(o))
                            .is_some()
                    })
                    .cloned()
                    .collect()
            });
        ensure!(!catalog.is_empty(), "请先同步 OpenRouter 模型价格");
        let display_name = |model: &str| {
            prices
                .iter()
                .find(|p| p.id == model && !p.name.is_empty())
                .map(|p| p.name.clone())
                .or_else(|| crate::qoder_state::model_display_name(model).map(str::to_owned))
                .unwrap_or_default()
        };
        let mut models = std::collections::BTreeMap::<String, String>::new();
        if let Some(model) = params.get("model") {
            let model = model
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("请输入模型 ID")?;
            models.insert(
                model.into(),
                params["name"]
                    .as_str()
                    .filter(|name| !name.is_empty() && *name != model)
                    .map(str::to_owned)
                    .unwrap_or_else(|| display_name(model)),
            );
        } else {
            let mut query = self.store.db.prepare("SELECT DISTINCT model FROM events")?;
            for model in query.query_map([], |r| r.get::<_, String>(0))? {
                let model = model?;
                let name = display_name(&model);
                models.insert(model, name);
            }
            // Include custom catalog entries that have not appeared in history yet.
            for price in &prices {
                if !catalog.iter().any(|p| p.id == price.id) {
                    models.entry(price.id.clone()).or_insert(price.name.clone());
                }
            }
        }
        let mut matches = Vec::new();
        let mut unmatched = Vec::new();
        let mut skipped = 0;
        for (model, name) in models {
            if settings.model_mappings.contains_key(&model) {
                skipped += 1;
                continue;
            }
            if let Some(price) = smart_reference(&model, &name, &catalog) {
                if model == price.id {
                    skipped += 1;
                    continue;
                }
                settings
                    .model_mappings
                    .insert(model.clone(), price.id.clone());
                matches.push(json!({"model":model,"priceId":price.id,"name":price.name}));
            } else {
                unmatched.push(model);
            }
        }
        if !matches.is_empty() {
            self.store.save_settings(&settings)?;
        }
        tx.commit()?;
        Ok(
            json!({"settings":self.store.settings()?,"matched":matches.len(),"matches":matches,"unmatched":unmatched,"skipped":skipped}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_effort_uses_an_unambiguous_base_tariff_across_model_families() {
        let prices = [
            "anthropic/claude-opus-5.5",
            "anthropic/claude-opus-5.5:batch",
            "google/gemini-4-neon",
            "openai/gpt-oss-120b",
            "openai/gpt-oss-120b-high",
            "custom/model",
            "openai/gpt-5.5",
            "other/gpt-5.5",
        ]
        .map(|id| ModelPrice {
            id: id.into(),
            ..Default::default()
        });
        let mut mappings = std::collections::BTreeMap::new();
        for (model, index) in [
            ("claude-opus-5-5-max", Some(0)),
            ("anthropic/claude-opus-5-5-low", Some(0)),
            ("gemini-4-neon-xhigh", Some(2)),
            ("gpt-oss-120b-medium", Some(3)),
            ("gpt-oss-120b-high", Some(4)),
            ("model-max", Some(5)),
            ("gpt-5-5-low", None),
            ("openai/gpt-5-5-low", Some(6)),
            ("claude-opus-5-5-preview", None),
            ("claude-opus-5-4-max", None),
            ("antigravity-unknown-5-high", None),
        ] {
            assert_eq!(
                find_usage_price("antigravity", model, &prices, &mappings).map(|p| &p.id),
                index.map(|i| &prices[i].id),
                "{model}"
            );
        }
        assert!(find_price("gpt-oss-120b-medium", &prices, &mappings).is_none());
        mappings.insert("claude-opus-5-5".into(), prices[5].id.clone());
        assert_eq!(
            find_usage_price("antigravity", "claude-opus-5-5-high", &prices, &mappings)
                .unwrap()
                .id,
            prices[5].id
        );
        mappings.insert("claude-opus-5-5-high".into(), "missing".into());
        assert!(
            find_usage_price("antigravity", "claude-opus-5-5-high", &prices, &mappings).is_none()
        );
    }

    #[test]
    fn gemini_effort_uses_the_base_tariff_after_exact_resolution() {
        let prices = [
            "google/gemini-4-hydrogen",
            "google/gemini-4-neon",
            "google/gemini-4-neon-high",
            "provider/other-model",
        ]
        .map(|id| ModelPrice {
            id: id.into(),
            ..Default::default()
        });
        let mut mappings = std::collections::BTreeMap::new();
        for (model, expected) in [
            ("gemini-4-hydrogen-medium", Some(&prices[0])),
            ("gemini-4-neon-low", Some(&prices[1])),
            ("google/gemini-4-neon-minimal", Some(&prices[1])),
            ("gemini-4-neon-high", Some(&prices[2])),
            ("gemini-4-neon-preview", None),
            ("gemini-4-unknown-high", None),
            ("other-model-high", None),
        ] {
            assert_eq!(
                find_price(model, &prices, &mappings).map(|p| &p.id),
                expected.map(|p| &p.id),
                "{model}"
            );
        }
        mappings.insert("gemini-4-neon".into(), prices[3].id.clone());
        assert_eq!(
            find_price("gemini-4-neon-medium", &prices, &mappings)
                .unwrap()
                .id,
            prices[3].id
        );
        mappings.insert("gemini-4-neon-medium".into(), prices[0].id.clone());
        assert_eq!(
            find_price("gemini-4-neon-medium", &prices, &mappings)
                .unwrap()
                .id,
            prices[0].id
        );
    }

    #[test]
    fn openrouter_sync_uses_the_last_catalog_and_retries_failed_attempts() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let synced = 1_800_000_000;
        assert!(store.openrouter_sync_due(synced).unwrap());
        let manual = ModelPrice {
            id: "manual/model".into(),
            fetched_at: synced,
            ..Default::default()
        };
        store.save_prices(&[manual]).unwrap();
        assert!(store.openrouter_sync_due(synced).unwrap());

        let catalog = ModelPrice {
            id: "provider/model".into(),
            fetched_at: synced,
            ..Default::default()
        };
        store.save_openrouter_prices(&[catalog]).unwrap();
        let due = synced + OPENROUTER_SYNC_INTERVAL;
        assert!(!store.openrouter_sync_due(due - 1).unwrap());
        assert!(store.openrouter_sync_due(due).unwrap());
        store
            .db
            .execute(
                "INSERT INTO kv(key,value) VALUES('openrouter-sync-attempt',?1)",
                [due.to_string()],
            )
            .unwrap();
        drop(store);
        let store = Store::open(root.path()).unwrap();
        assert!(
            !store
                .openrouter_sync_due(due + OPENROUTER_RETRY_INTERVAL - 1)
                .unwrap()
        );
        assert!(
            store
                .openrouter_sync_due(due + OPENROUTER_RETRY_INTERVAL)
                .unwrap()
        );
    }

    #[test]
    fn openrouter_catalog_write_failure_preserves_the_previous_prices() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let original = ModelPrice {
            id: "provider/model".into(),
            input: Some(1e-6),
            fetched_at: 100,
            ..Default::default()
        };
        store
            .save_openrouter_prices(std::slice::from_ref(&original))
            .unwrap();
        store
            .db
            .execute_batch(
                "CREATE TRIGGER reject_openrouter BEFORE INSERT ON kv
                WHEN NEW.key='openrouter-models'
                BEGIN SELECT RAISE(ABORT,'fixture failure'); END;",
            )
            .unwrap();
        let updated = ModelPrice {
            input: Some(2e-6),
            fetched_at: 200,
            ..original
        };
        assert!(store.save_openrouter_prices(&[updated]).is_err());
        let prices = store.prices().unwrap();
        assert_eq!(prices[0].input, Some(1e-6));
        assert!(
            store
                .openrouter_sync_due(100 + OPENROUTER_SYNC_INTERVAL)
                .unwrap()
        );
    }

    #[test]
    fn missing_cache_price_does_not_count_as_free() {
        let p = ModelPrice {
            input: Some(2.0 / 1e6),
            output: Some(10.0 / 1e6),
            ..Default::default()
        };
        let t = Tokens {
            input: 1_000_000,
            output: 100_000,
            cache_read: 900_000,
            ..Default::default()
        };
        assert_eq!(estimate(&t, Some(&p)), (3.0, 1_100_000));
    }
    #[test]
    fn exact_mapping_only() {
        let prices = vec![ModelPrice {
            id: "openai/gpt-test".into(),
            ..Default::default()
        }];
        assert!(find_price("gpt-test", &prices, &Default::default()).is_some());
        assert!(find_price("gpt-test-latest", &prices, &Default::default()).is_none());
    }
    #[test]
    fn smart_references_preserve_versions_variants_and_prefer_authors() {
        let prices: Vec<ModelPrice> = [
            ("relay/claude-sonnet-4.5", "Claude Sonnet 4.5"),
            (
                "anthropic/claude-sonnet-4.5",
                "Anthropic: Claude Sonnet 4.5",
            ),
            ("anthropic/claude-sonnet-4.5:batch", "Claude Sonnet 4.5"),
            ("anthropic/claude-sonnet-4.6", "Claude Sonnet 4.6"),
            ("anthropic/claude-sonnet-latest", "Claude Sonnet Latest"),
            ("anthropic/claude-sonnet-4.5-20250929", "Claude Sonnet 4.5"),
            ("openai/gpt-4.1", "OpenAI: GPT-4.1"),
            ("openai/gpt-4.1:free", "GPT-4.1 Free"),
            ("google/gemini-2.5-pro", "Google: Gemini 2.5 Pro"),
            ("deepseek/deepseek-v3.2", "DeepSeek V3.2"),
        ]
        .into_iter()
        .map(|(id, name)| ModelPrice {
            id: id.into(),
            name: name.into(),
            input: Some(0.000002),
            output: Some(0.00001),
            ..Default::default()
        })
        .collect();
        for (id, name, expected) in [
            (
                "relay/claude-sonnet-4.5",
                "",
                Some("anthropic/claude-sonnet-4.5"),
            ),
            (
                "mystery/Claude-4-5-Sonnet",
                "",
                Some("anthropic/claude-sonnet-4.5"),
            ),
            (
                "deployment",
                "Anthropic: Claude Sonnet 4.5",
                Some("anthropic/claude-sonnet-4.5"),
            ),
            ("claude-sonnet", "", Some("anthropic/claude-sonnet-latest")),
            (
                "claude-sonnet",
                "Claude Sonnet 4.5",
                Some("anthropic/claude-sonnet-4.5"),
            ),
            (
                "claude-sonnet-latest",
                "",
                Some("anthropic/claude-sonnet-latest"),
            ),
            (
                "claude-sonnet-4.5:batch",
                "",
                Some("anthropic/claude-sonnet-4.5:batch"),
            ),
            (
                "claude-sonnet-4.5-2025-09-29",
                "",
                Some("anthropic/claude-sonnet-4.5-20250929"),
            ),
            ("claude-sonnet-4.5-20250930", "Claude Sonnet 4.5", None),
            ("claude-sonnet-4.7", "Claude Sonnet", None),
            ("claude-sonnet-4.7", "Claude Sonnet 4.5", None),
            ("azure/gpt-4-1", "", Some("openai/gpt-4.1")),
            ("external/DeepSeek-V3.2", "", Some("deepseek/deepseek-v3.2")),
            ("Gemini-2.5-Pro", "", Some("google/gemini-2.5-pro")),
            ("gemini-2.5-flash", "", None),
            ("antigravity-unknown-123", "GPT-4.1", None),
        ] {
            assert_eq!(
                smart_reference(id, name, &prices).map(|p| p.id.as_str()),
                expected,
                "{id} / {name}"
            );
        }
        // A missing version must not select an arbitrary dated/numbered release.
        assert!(smart_reference("gpt", "", &prices).is_none());
        let batch_only = vec![prices[2].clone()];
        assert!(smart_reference("claude-sonnet-4.5", "", &batch_only).is_none());
        assert!(smart_reference("claude-sonnet-4.5:batch", "", &prices[0..2]).is_none());
        // Smart references are opt-in; ordinary resolution remains exact.
        assert!(find_price("azure/gpt-4-1", &prices, &Default::default()).is_none());
    }
}
