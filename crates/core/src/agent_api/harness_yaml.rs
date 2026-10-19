//! Preserve native YAML tags without evaluating them. serde_yaml_ng otherwise
//! drops non-core global tags such as !!js before a Value can retain them.
use anyhow::{Context, Result, ensure};
use serde_yaml_ng::{Value, value::Tag};
use std::{collections::HashMap, ffi::CStr, mem::MaybeUninit};
use unsafe_libyaml as unsafe_yaml;

const ENCODED: &str = "!aieyes-harness-uri-";
struct TaggedSpan {
    start: usize,
    end: usize,
    uri: String,
    scalar: Option<String>,
}
struct Scanner(Box<MaybeUninit<unsafe_yaml::yaml_parser_t>>);
impl Drop for Scanner {
    fn drop(&mut self) {
        // Initialized by scan; input is still alive until scan returns.
        unsafe { unsafe_yaml::yaml_parser_delete(self.0.as_mut_ptr()) };
    }
}
struct Token(unsafe_yaml::yaml_token_t);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe { unsafe_yaml::yaml_token_delete(&mut self.0) };
    }
}
fn cstring(ptr: *const u8) -> Result<String> {
    ensure!(!ptr.is_null(), "Harness YAML 标签不可用");
    // These are NUL-terminated strings owned by a live libyaml token.
    let value = unsafe { CStr::from_ptr(ptr.cast()) }
        .to_str()
        .context("Harness YAML 标签编码无效")?;
    ensure!(value.len() <= 4096, "Harness YAML 标签过长");
    Ok(value.to_owned())
}
fn scan(input: &str) -> Result<Vec<TaggedSpan>> {
    let mut raw = Box::<unsafe_yaml::yaml_parser_t>::new_uninit();
    // Heap allocation, initialization, and token lifetime follow libyaml's
    // scanner API. It only reads input; every successful token is deleted.
    ensure!(
        !unsafe { unsafe_yaml::yaml_parser_initialize(raw.as_mut_ptr()) }.fail,
        "无法初始化 Harness YAML 读取器"
    );
    let mut scanner = Scanner(raw);
    unsafe {
        unsafe_yaml::yaml_parser_set_input_string(
            scanner.0.as_mut_ptr(),
            input.as_ptr(),
            input.len() as u64,
        )
    };
    let mut handles = HashMap::from([
        ("!".to_owned(), "!".to_owned()),
        ("!!".to_owned(), "tag:yaml.org,2002:".to_owned()),
    ]);
    let mut tags: Vec<TaggedSpan> = Vec::new();
    let mut pending = None;
    loop {
        let mut token = MaybeUninit::uninit();
        ensure!(
            !unsafe { unsafe_yaml::yaml_parser_scan(scanner.0.as_mut_ptr(), token.as_mut_ptr()) }
                .fail,
            "Harness 配置不是有效 YAML；未覆盖配置"
        );
        let token = Token(unsafe { token.assume_init() });
        match token.0.type_ {
            unsafe_yaml::YAML_STREAM_END_TOKEN => break,
            unsafe_yaml::YAML_TAG_DIRECTIVE_TOKEN => {
                let directive = unsafe { token.0.data.tag_directive };
                handles.insert(cstring(directive.handle)?, cstring(directive.prefix)?);
            }
            unsafe_yaml::YAML_TAG_TOKEN => {
                let tag = unsafe { token.0.data.tag };
                let handle = cstring(tag.handle)?;
                let suffix = cstring(tag.suffix)?;
                let uri = if handle.is_empty() {
                    suffix
                } else {
                    format!(
                        "{}{suffix}",
                        handles
                            .get(&handle)
                            .context("Harness YAML 标签前缀未定义")?
                    )
                };
                let start = token.0.start_mark.index as usize;
                let end = token.0.end_mark.index as usize;
                ensure!(input.get(start..end).is_some(), "Harness YAML 标签位置无效");
                tags.push(TaggedSpan {
                    start,
                    end,
                    uri,
                    scalar: None,
                });
                pending = Some(tags.len() - 1);
            }
            unsafe_yaml::YAML_ANCHOR_TOKEN => {}
            unsafe_yaml::YAML_SCALAR_TOKEN => {
                if let Some(index) = pending.take() {
                    let scalar = unsafe { token.0.data.scalar };
                    let bytes =
                        unsafe { std::slice::from_raw_parts(scalar.value, scalar.length as usize) };
                    tags[index].scalar = Some(
                        std::str::from_utf8(bytes)
                            .context("Harness YAML 值编码无效")?
                            .to_owned(),
                    );
                }
            }
            _ => pending = None,
        }
    }
    Ok(tags)
}
fn builtin(uri: &str) -> bool {
    uri.strip_prefix("tag:yaml.org,2002:")
        .is_some_and(|s| matches!(s, "str" | "bool" | "int" | "float" | "null" | "seq" | "map"))
        || uri == "!"
}
fn splice(input: &str, edits: &[(usize, usize, String)]) -> String {
    let mut output = String::new();
    let mut offset = 0;
    for (start, end, replacement) in edits {
        output.push_str(&input[offset..*start]);
        output.push_str(replacement);
        offset = *end;
    }
    output.push_str(&input[offset..]);
    output
}
fn hex(value: &str) -> String {
    value.bytes().map(|b| format!("{b:02x}")).collect()
}

pub(super) fn parse(bytes: &[u8]) -> Result<Value> {
    let input = std::str::from_utf8(bytes).context("Harness YAML 应为 UTF-8")?;
    let tags = scan(input)?;
    let mut edits = Vec::new();
    let mut metadata = HashMap::new();
    for (index, tag) in tags
        .iter()
        .enumerate()
        .filter(|(_, tag)| !builtin(&tag.uri))
    {
        let temporary = format!("!aieyes-read-tag-{index}");
        edits.push((tag.start, tag.end, temporary.clone()));
        metadata.insert(temporary, tag);
    }
    let mut value = serde_yaml_ng::from_str(&splice(input, &edits))
        .map_err(|_| anyhow::anyhow!("Harness 配置不是有效 YAML；未覆盖配置"))?;
    fn restore(value: &mut Value, metadata: &HashMap<String, &TaggedSpan>) {
        match value {
            Value::Tagged(tagged) => {
                if let Some(original) = metadata.get(&tagged.tag.to_string()) {
                    tagged.tag = Tag::new(format!("{ENCODED}{}", hex(&original.uri)));
                    if let Some(scalar) = &original.scalar {
                        tagged.value = Value::String(scalar.clone());
                    }
                }
                restore(&mut tagged.value, metadata);
            }
            Value::Sequence(rows) => rows.iter_mut().for_each(|v| restore(v, metadata)),
            Value::Mapping(map) => {
                let previous = std::mem::take(map);
                for (mut key, mut value) in previous {
                    restore(&mut key, metadata);
                    restore(&mut value, metadata);
                    map.insert(key, value);
                }
            }
            _ => {}
        }
    }
    restore(&mut value, &metadata);
    Ok(value)
}
pub(super) fn encode(value: &Value) -> Result<Vec<u8>> {
    let encoded = serde_yaml_ng::to_string(value)?;
    let mut edits = Vec::new();
    for tag in scan(&encoded)? {
        if let Some(hex) = tag.uri.strip_prefix(ENCODED) {
            ensure!(
                hex.len() % 2 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "Harness YAML 标签编码无效"
            );
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("Harness YAML 标签编码无效")?;
            let uri = std::str::from_utf8(&bytes).context("Harness YAML 标签编码无效")?;
            let replacement = if let Some(suffix) =
                uri.strip_prefix("tag:yaml.org,2002:").filter(|s| {
                    !s.is_empty()
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                }) {
                format!("!!{suffix}")
            } else {
                let escaped: String = uri
                    .bytes()
                    .map(|b| {
                        if b.is_ascii_alphanumeric() || b"-._~:/,;!@$&'()*+=?#[]".contains(&b) {
                            (b as char).to_string()
                        } else {
                            format!("%{b:02X}")
                        }
                    })
                    .collect();
                format!("!<{escaped}>")
            };
            edits.push((tag.start, tag.end, replacement));
        }
    }
    Ok(splice(&encoded, &edits).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_tags_scalars_aliases_unicode_and_literal_lookalikes() {
        let input = "%TAG !x! tag:example.test,2026:\n---\n- id: session-persistence-jsonl\n  config:\n    root: &root !!js dshHomePath('sessions')\n    same: *root\n    numeric: !!js 010\n    bool: !!js true\n    list: !x!list [1, false]\n    local: !example abc\n    text: '文字 !!js not a tag'\n    block: |\n      !!js not a tag\n    mapping: !<tag:example.test,2026:map> {key: !!str true}\n    builtin: !!int 10\n";
        let parsed = parse(input.as_bytes()).unwrap();
        let encoded = encode(&parsed).unwrap();
        assert_eq!(parse(&encoded).unwrap(), parsed);
        let text = String::from_utf8(encoded).unwrap();
        assert!(text.contains("!!js"));
        assert!(text.contains("010"));
        assert!(text.contains("文字 !!js not a tag"));
        let config = &parsed[0]["config"];
        assert!(matches!(&config["root"], Value::Tagged(_)));
        assert_eq!(config["root"], config["same"]);
        assert!(
            matches!(&config["numeric"],Value::Tagged(t) if t.value==Value::String("010".into()))
        );
        assert_eq!(config["mapping"]["key"].as_str(), Some("true"));
        assert_eq!(config["builtin"].as_i64(), Some(10));
    }
    #[test]
    fn invalid_yaml_and_tags_fail_without_disclosing_content() {
        for input in [
            "[PRIVATE_SECRET",
            "x: !missing!PRIVATE_SECRET 1",
            "x: !!js !!js PRIVATE_SECRET",
        ] {
            assert!(
                !parse(input.as_bytes())
                    .unwrap_err()
                    .to_string()
                    .contains("PRIVATE_SECRET")
            );
        }
    }
}
