//! Presentation-only translations. Domain data and protocol identifiers are unchanged.
use std::{borrow::Cow, collections::HashMap, sync::OnceLock};

struct Pattern {
    chunks: Vec<String>,
    slots: Vec<usize>,
    target: String,
}

struct Catalog {
    exact: HashMap<String, String>,
    patterns: Vec<Pattern>,
}

pub fn normalize(language: &str) -> &'static str {
    if language == "en" { "en" } else { "zh-CN" }
}

fn parameters(template: &str) -> Vec<(usize, usize, usize)> {
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(start) = template[offset..].find('{') {
        let start = offset + start;
        let Some(end) = template[start..].find('}') else {
            break;
        };
        let end = start + end;
        if let Ok(slot) = template[start + 1..end].parse::<usize>() {
            result.push((start, end + 1, slot));
        }
        offset = end + 1;
    }
    result
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let exact: HashMap<String, String> =
            serde_json::from_str(include_str!("../../../locales/en.json"))
                .expect("bundled localization catalogue is valid JSON");
        let mut patterns = Vec::new();
        for (source, target) in &exact {
            let positions = parameters(source);
            if positions.is_empty() {
                continue;
            }
            let mut chunks = Vec::new();
            let mut slots = Vec::new();
            let mut offset = 0;
            for (start, end, slot) in positions {
                chunks.push(source[offset..start].to_owned());
                slots.push(slot);
                offset = end;
            }
            chunks.push(source[offset..].to_owned());
            patterns.push(Pattern {
                chunks,
                slots,
                target: target.clone(),
            });
        }
        // Prefer the most specific complete sentence to a generic status pattern.
        patterns
            .sort_by_key(|p| std::cmp::Reverse(p.chunks.iter().map(String::len).sum::<usize>()));
        Catalog { exact, patterns }
    })
}

pub fn label(language: &str, source: &'static str) -> &'static str {
    if normalize(language) == "en" {
        catalog()
            .exact
            .get(source)
            .map(String::as_str)
            .unwrap_or(source)
    } else {
        source
    }
}

fn capture<'a>(pattern: &Pattern, source: &'a str) -> Option<Vec<(usize, &'a str)>> {
    let mut rest = source.strip_prefix(&pattern.chunks[0])?;
    let mut values = Vec::new();
    for (index, slot) in pattern.slots.iter().enumerate() {
        let next = &pattern.chunks[index + 1];
        let end = if index + 1 == pattern.slots.len() {
            rest.strip_suffix(next)?.len()
        } else {
            rest.find(next)?
        };
        values.push((*slot, &rest[..end]));
        rest = &rest[end + next.len()..];
    }
    rest.is_empty().then_some(values)
}

fn substitute(template: &str, values: &[(usize, &str)]) -> String {
    let mut output = String::new();
    let mut offset = 0;
    for (start, end, slot) in parameters(template) {
        output.push_str(&template[offset..start]);
        output.push_str(
            values
                .iter()
                .find(|(key, _)| *key == slot)
                .map(|(_, value)| *value)
                .unwrap_or(&template[start..end]),
        );
        offset = end;
    }
    output.push_str(&template[offset..]);
    output
}

pub fn translate<'a>(language: &str, source: &'a str) -> Cow<'a, str> {
    if normalize(language) != "en" {
        return Cow::Borrowed(source);
    }
    if let Some(exact) = catalog().exact.get(source) {
        return Cow::Borrowed(exact);
    }
    if !source
        .chars()
        .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
    {
        return Cow::Borrowed(source);
    }
    for pattern in &catalog().patterns {
        if let Some(values) = capture(pattern, source) {
            return Cow::Owned(substitute(&pattern.target, &values));
        }
    }
    if source.contains('\n') {
        return Cow::Owned(
            source
                .split('\n')
                .map(|line| translate(language, line))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    Cow::Borrowed(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sentences_preserve_arguments_and_do_not_translate_substrings() {
        assert_eq!(label("en", "本机"), "Local");
        assert_eq!(label("zh-CN", "本机"), "本机");
        assert_eq!(
            translate("en", "新增 2 个账户、3 个连接"),
            "Added 2 accounts and 3 connections"
        );
        assert_eq!(
            translate("en", "切换为此账户 · 用户中文名称"),
            "Switch account · 用户中文名称"
        );
        assert_eq!(translate("en", "my-本机-project"), "my-本机-project");
        assert_eq!(normalize("fr"), "zh-CN");
        assert_eq!(
            substitute("{1} / {0}", &[(0, "{1}"), (1, "路径")]),
            "路径 / {1}"
        );
    }
}
