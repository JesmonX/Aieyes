use crate::models::*;
use serde_json::Value;

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
    if crate::antigravity::unknown_model(model) {
        return None;
    }
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
pub fn smart_reference<'a>(
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
