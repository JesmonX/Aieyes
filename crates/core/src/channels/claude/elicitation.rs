//! MCP elicitation forms held by the `Elicitation` hook of Claude Code / Qoder CLI.
//! Only form mode with a flat schema of primitive fields is shown; anything else (URL mode,
//! nested objects) stays in the terminal's own dialog. The card and the reply are both derived
//! from the stored request, so a reply can only fill fields the MCP server asked for.
use super::PendingClaude;
use crate::channels::{Session, content};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

const METHOD: &str = "aieyes/claudeElicitation";
const MAX_FIELDS: usize = 12;
const MAX_OPTIONS: usize = 50;
pub(super) const MAX_SCHEMA: usize = 64 * 1024;

struct Field<'a> {
    key: &'a str,
    /// `string` | `number` | `integer` | `boolean` | `multiselect`.
    kind: &'static str,
    schema: &'a Value,
    /// `(value, label)`; empty for free input.
    options: Vec<(Value, String)>,
    required: bool,
}

fn titled(items: &Value) -> Option<Vec<(Value, String)>> {
    items
        .as_array()?
        .iter()
        .map(|item| {
            let value = item["const"].as_str()?;
            Some((
                json!(value),
                item["title"].as_str().unwrap_or(value).to_owned(),
            ))
        })
        .collect()
}

fn listed(schema: &Value) -> Option<Vec<(Value, String)>> {
    let values = schema["enum"].as_array()?;
    let names = schema["enumNames"].as_array();
    values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let value = value.as_str()?;
            let label = names
                .and_then(|names| names.get(i)?.as_str())
                .unwrap_or(value);
            Some((json!(value), label.to_owned()))
        })
        .collect()
}

fn choices(schema: &Value) -> Option<Vec<(Value, String)>> {
    let options = if schema["enum"].is_array() {
        listed(schema)?
    } else if schema["oneOf"].is_array() {
        titled(&schema["oneOf"])?
    } else if schema["anyOf"].is_array() {
        titled(&schema["anyOf"])?
    } else {
        Vec::new()
    };
    (options.len() <= MAX_OPTIONS).then_some(options)
}

/// The keys of a JSON object in the order they were written. Parsed JSON objects are sorted
/// by key, so a form's field order has to be read from the text itself.
#[derive(Default)]
pub(in crate::channels) struct Keys(pub Vec<String>);

impl<'de> serde::Deserialize<'de> for Keys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, Visitor};
        struct Collect;
        impl<'de> Visitor<'de> for Collect {
            type Value = Keys;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
                let mut keys = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    map.next_value::<IgnoredAny>()?;
                    keys.push(key);
                }
                Ok(Keys(keys))
            }
        }
        deserializer.deserialize_map(Collect)
    }
}

/// The property names of `requested_schema` in the order the MCP server listed them, read
/// from the hook input itself.
pub(super) fn field_order(hook_input: &[u8]) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Schema {
        #[serde(default)]
        properties: Keys,
    }
    #[derive(serde::Deserialize)]
    struct Hook {
        requested_schema: Schema,
    }
    serde_json::from_slice::<Hook>(hook_input)
        .map(|hook| hook.requested_schema.properties.0)
        .unwrap_or_default()
}

/// The fields of a stored form, or `None` when any part of it cannot be shown faithfully.
fn fields(stored: &Value) -> Option<Vec<Field<'_>>> {
    let schema = &stored["requested_schema"];
    if schema["type"] != "object" {
        return None;
    }
    let properties = schema["properties"].as_object()?;
    if properties.is_empty() || properties.len() > MAX_FIELDS {
        return None;
    }
    let required = schema["required"].as_array();
    let position = |key: &String| {
        stored["order"]
            .as_array()
            .and_then(|order| order.iter().position(|name| name == key.as_str()))
            .unwrap_or(usize::MAX)
    };
    let mut listed: Vec<_> = properties.iter().collect();
    listed.sort_by_key(|(key, _)| position(key));
    listed
        .into_iter()
        .map(|(key, prop)| {
            if key.is_empty() || key.len() > 200 || !prop.is_object() {
                return None;
            }
            let (kind, options) = match prop["type"].as_str()? {
                "string" => ("string", choices(prop)?),
                "number" => ("number", Vec::new()),
                "integer" => ("integer", Vec::new()),
                "boolean" => ("boolean", Vec::new()),
                "array" => {
                    let options = choices(&prop["items"])?;
                    if options.is_empty() {
                        return None;
                    }
                    ("multiselect", options)
                }
                _ => return None,
            };
            Some(Field {
                key,
                kind,
                schema: prop,
                options,
                required: required.is_some_and(|names| names.iter().any(|n| n == key.as_str())),
            })
        })
        .collect()
}

/// What the helper stores for a form it can show; `None` leaves the form to the terminal.
pub(in crate::channels) fn stored(hook: &Value, order: Vec<String>) -> Option<Value> {
    if !matches!(hook["mode"].as_str(), None | Some("form")) {
        return None;
    }
    let schema = &hook["requested_schema"];
    if schema.to_string().len() > MAX_SCHEMA {
        return None;
    }
    let stored = json!({
        "message": content::clip(hook["message"].as_str().unwrap_or(""), 4000),
        "server": content::clip(hook["mcp_server_name"].as_str().unwrap_or(""), 200),
        "requested_schema": schema,
        "order": order,
    });
    fields(&stored)?;
    Some(stored)
}

/// The question rows of a stored form (see `stored`), for any adapter that holds one.
pub(in crate::channels) fn questions(stored: &Value, lang: &str) -> Option<Vec<Value>> {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let fields = fields(stored)?;
    let questions = fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let text = |key: &str| field.schema[key].as_str().unwrap_or("").trim();
            let (title, description) = (text("title"), text("description"));
            let options: Vec<Value> = if field.kind == "boolean" {
                vec![
                    json!({"label": tr("是"), "value": true}),
                    json!({"label": tr("否"), "value": false}),
                ]
            } else {
                field
                    .options
                    .iter()
                    .map(|(value, label)| json!({"label": label, "value": value}))
                    .collect()
            };
            let mut constraints = Map::new();
            for key in ["minimum", "maximum", "minLength", "maxLength", "format"] {
                if !field.schema[key].is_null() {
                    constraints.insert(key.into(), field.schema[key].clone());
                }
            }
            let question = [description, title, field.key]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("");
            let header = if description.is_empty() { "" } else { title };
            json!({
                "id": (i + 1).to_string(),
                "key": field.key,
                "kind": field.kind,
                "header": header,
                "question": question,
                "options": options,
                "multiSelect": field.kind == "multiselect",
                "allowCustom": field.options.is_empty() && field.kind != "boolean",
                "required": field.required,
                "default": field.schema["default"],
                "constraints": constraints,
            })
        })
        .collect::<Vec<_>>();
    Some(questions)
}

/// The question card of a held form.
pub(crate) fn request(target: &Session, req: &PendingClaude, lang: &str) -> Option<Value> {
    let tr = |t: &str| crate::localization::translate(lang, t).into_owned();
    let questions = questions(&req.input, lang)?;
    let server = req.input["server"].as_str().unwrap_or("");
    let title = if server.is_empty() {
        tr("MCP 表单")
    } else {
        server.to_owned()
    };
    Some(json!({"method": METHOD, "params": {
        "threadId": target.session.native_session,
        "turnId": target.session.turn_id,
        "itemId": req.call_id,
        "form": {
            "title": title,
            "description": req.input["message"],
            "source": "mcp",
            "serverName": server,
            "canCancel": true,
            "canSkip": true,
        },
        "questions": questions,
    }}))
}

fn pick(field: &Field, answer: &str) -> Result<Value> {
    let answer = answer.trim();
    field
        .options
        .iter()
        .find(|(value, label)| {
            value.as_str() == Some(answer) || label.trim().eq_ignore_ascii_case(answer)
        })
        .map(|(value, _)| value.clone())
        .context("回答不在可选项中，请重新选择")
}

fn number(field: &Field, raw: &Value) -> Result<Value> {
    let value = match raw {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
    .context("请回复一个数字")?;
    ensure!(
        field.schema["minimum"].as_f64().is_none_or(|m| value >= m)
            && field.schema["maximum"].as_f64().is_none_or(|m| value <= m),
        "数值超出此表单允许的范围"
    );
    if field.kind == "integer" {
        ensure!(
            value.fract() == 0.0 && value.abs() < 9.0e15,
            "请回复一个整数"
        );
        return Ok(json!(value as i64));
    }
    Ok(json!(value))
}

fn coerce(field: &Field, raw: &Value) -> Result<Value> {
    match field.kind {
        "boolean" => match raw {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(s) => match s.trim().to_lowercase().as_str() {
                "true" | "yes" | "y" | "是" => Ok(json!(true)),
                "false" | "no" | "n" | "否" => Ok(json!(false)),
                _ => bail!("回答不在可选项中，请重新选择"),
            },
            _ => bail!("回答不在可选项中，请重新选择"),
        },
        "number" | "integer" => number(field, raw),
        "multiselect" => {
            let picked: Vec<&str> = match raw {
                Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
                // One label, or a comma separated list of them.
                Value::String(s) if pick(field, s).is_ok() => vec![s.as_str()],
                Value::String(s) => s.split([',', '，']).collect(),
                _ => bail!("回答不在可选项中，请重新选择"),
            };
            let mut values = Vec::new();
            for answer in picked.into_iter().filter(|s| !s.trim().is_empty()) {
                let value = pick(field, answer)?;
                if !values.contains(&value) {
                    values.push(value);
                }
            }
            let count = values.len() as u64;
            ensure!(
                field.schema["minItems"].as_u64().is_none_or(|m| count >= m)
                    && field.schema["maxItems"].as_u64().is_none_or(|m| count <= m),
                "选择的数量不符合此表单的要求"
            );
            Ok(Value::Array(values))
        }
        _ => {
            let text = match raw {
                Value::String(s) => s.trim().to_owned(),
                Value::Number(n) => n.to_string(),
                _ => bail!("问题回答不能为空"),
            };
            if !field.options.is_empty() {
                return pick(field, &text);
            }
            let length = text.chars().count() as u64;
            ensure!(
                field.schema["minLength"]
                    .as_u64()
                    .is_none_or(|m| length >= m)
                    && field.schema["maxLength"]
                        .as_u64()
                        .is_none_or(|m| length <= m),
                "回答长度不符合此表单的要求"
            );
            Ok(json!(text))
        }
    }
}

fn empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// The `content` object of an accepted form: typed values for the fields that were asked.
pub(in crate::channels) fn content(
    stored: &Value,
    answers: Option<&Value>,
    text: Option<&str>,
) -> Result<Value> {
    let fields = fields(stored).context("原会话问题已不可用")?;
    let single = text
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| json!(s));
    let map = answers.and_then(Value::as_object);
    ensure!(
        map.is_some() || (fields.len() == 1 && single.is_some()),
        "请回答所有问题后再提交"
    );
    let mut out = Map::new();
    for (i, field) in fields.iter().enumerate() {
        let title = field.schema["title"].as_str().unwrap_or("");
        let description = field.schema["description"].as_str().unwrap_or("");
        let raw = match map {
            Some(map) => [(i + 1).to_string().as_str(), field.key, description, title]
                .into_iter()
                .filter(|k| !k.is_empty())
                .find_map(|k| map.get(k)),
            None => single.as_ref(),
        };
        match raw.filter(|v| !empty(v)) {
            Some(raw) => {
                out.insert(field.key.to_owned(), coerce(field, raw)?);
            }
            None => ensure!(!field.required, "请回答所有问题后再提交"),
        }
    }
    Ok(Value::Object(out))
}

pub(super) fn payload(action: &str, content: Option<Value>) -> Value {
    let mut output = json!({"hookEventName": "Elicitation", "action": action});
    if let Some(content) = content {
        output["content"] = content;
    }
    json!({"hookSpecificOutput": output})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook_text() -> &'static str {
        r#"{
            "hook_event_name": "Elicitation",
            "mcp_server_name": "deploy",
            "message": "Where should this go?",
            "mode": "form",
            "elicitation_id": "el-1",
            "requested_schema": {"type": "object", "required": ["env", "replicas"], "properties": {
                "env": {"type": "string", "title": "Environment", "enum": ["prod", "stg"], "enumNames": ["Production", "Staging"]},
                "replicas": {"type": "integer", "description": "How many replicas", "title": "Replicas", "minimum": 1, "maximum": 5, "default": 2},
                "notify": {"type": "boolean", "title": "Notify the team"},
                "regions": {"type": "array", "title": "Regions", "maxItems": 2,
                    "items": {"anyOf": [{"const": "us", "title": "US"}, {"const": "eu", "title": "EU"}, {"const": "ap"}]}},
                "note": {"type": "string", "title": "Note", "maxLength": 10}
            }}
        }"#
    }

    fn hook() -> Value {
        serde_json::from_str(hook_text()).unwrap()
    }

    #[test]
    fn only_flat_form_schemas_are_held() {
        let order = field_order(hook_text().as_bytes());
        assert_eq!(order, ["env", "replicas", "notify", "regions", "note"]);
        assert!(
            field_order(b"{\"requested_schema\":{}}").is_empty() && field_order(b"[]").is_empty()
        );
        assert!(stored(&hook(), order).is_some());
        let mut url = hook();
        url["mode"] = json!("url");
        assert!(stored(&url, Vec::new()).is_none());
        for bad in [
            json!({"type": "object", "properties": {}}),
            json!({"type": "string"}),
            json!({"type": "object", "properties": {"a": {"type": "object", "properties": {}}}}),
            json!({"type": "object", "properties": {"a": {"type": "array", "items": {"type": "string"}}}}),
            json!({"type": "object", "properties": {"a": {"type": "string", "enum": [1, 2]}}}),
        ] {
            let mut hook = hook();
            hook["requested_schema"] = bad;
            assert!(
                stored(&hook, Vec::new()).is_none(),
                "{}",
                hook["requested_schema"]
            );
        }
        let mut large = hook();
        large["requested_schema"]["properties"]["env"]["description"] =
            json!("x".repeat(MAX_SCHEMA));
        assert!(stored(&large, Vec::new()).is_none());
    }

    #[test]
    fn answers_become_typed_content_and_are_checked_against_the_schema() {
        let stored = stored(&hook(), field_order(hook_text().as_bytes())).unwrap();
        let ok = content(
            &stored,
            Some(&json!({"1": "Production", "2": "3", "notify": true, "regions": ["US", "ap"], "Note": ""})),
            None,
        )
        .unwrap();
        assert_eq!(
            ok,
            json!({"env": "prod", "replicas": 3, "notify": true, "regions": ["us", "ap"]})
        );
        let ok = content(
            &stored,
            Some(
                &json!({"env": "stg", "How many replicas": 1, "notify": "否", "regions": "US, EU"}),
            ),
            None,
        )
        .unwrap();
        assert_eq!(
            ok,
            json!({"env": "stg", "replicas": 1, "notify": false, "regions": ["us", "eu"]})
        );
        for bad in [
            json!({"env": "prod"}),
            json!({"env": "dev", "replicas": 1}),
            json!({"env": "prod", "replicas": 9}),
            json!({"env": "prod", "replicas": 1.5}),
            json!({"env": "prod", "replicas": "many"}),
            json!({"env": "prod", "replicas": 1, "notify": "maybe"}),
            json!({"env": "prod", "replicas": 1, "regions": ["us", "eu", "ap"]}),
            json!({"env": "prod", "replicas": 1, "regions": ["moon"]}),
            json!({"env": "prod", "replicas": 1, "note": "longer than ten"}),
        ] {
            assert!(content(&stored, Some(&bad), None).is_err(), "{bad}");
        }
        // Plain text can only answer a form with a single field.
        assert!(content(&stored, None, Some("prod")).is_err());
        let mut one = hook();
        one["requested_schema"] =
            json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let one = super::stored(&one, Vec::new()).unwrap();
        assert_eq!(
            content(&one, None, Some(" Ada ")).unwrap(),
            json!({"name": "Ada"})
        );
        assert_eq!(
            payload("accept", Some(json!({"name": "Ada"}))),
            json!({"hookSpecificOutput": {"hookEventName": "Elicitation", "action": "accept", "content": {"name": "Ada"}}})
        );
        assert_eq!(
            payload("decline", None)["hookSpecificOutput"],
            json!({"hookEventName": "Elicitation", "action": "decline"})
        );
    }
}
