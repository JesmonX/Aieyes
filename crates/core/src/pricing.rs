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
}
