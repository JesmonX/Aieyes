//! Bounded, authenticated question-button callbacks. No agent/network I/O on the ACK path.
//! Single-select requests submit with the last choice; a multi-select question adds an
//! explicit submit button, because toggling an option cannot mean "finished".
use super::{codex, configuration, content, db};
use crate::{models::now, sessions::Phase, store::Store};
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub(super) fn setup(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS channel_card_actions (
        outbox_id TEXT PRIMARY KEY REFERENCES channel_outbox(id) ON DELETE CASCADE,
        request TEXT NOT NULL, choices TEXT NOT NULL DEFAULT '{}', completed INTEGER NOT NULL DEFAULT 0) WITHOUT ROWID;")?;
    Ok(())
}

fn questions(request: &Value) -> Option<&Vec<Value>> {
    if !matches!(
        request["method"].as_str(),
        Some(
            "aieyes/asyncQuestion"
                | "aieyes/claudeQuestion"
                | "item/tool/requestUserInput"
                | "tool/requestUserInput"
        )
    ) || request.to_string().len() > 16000
    {
        return None;
    }
    let q = request["params"]["questions"].as_array()?;
    if q.is_empty() || q.len() > 8 {
        return None;
    }
    let mut total = 0;
    for item in q {
        if item["multiSelect"] == true && request["method"] != "aieyes/claudeQuestion" {
            return None;
        }
        let id = item["id"].as_str()?;
        if id.is_empty() || id.len() > 256 || id.contains(['\n', '\r', '=']) {
            return None;
        }
        let options = item["options"].as_array()?;
        if options.is_empty()
            || options
                .iter()
                .any(|o| o["label"].as_str().is_none_or(str::is_empty))
        {
            return None;
        }
        total += options.len();
    }
    (total <= 24).then_some(q)
}

fn manual(questions: &[Value]) -> bool {
    questions.iter().any(|q| q["multiSelect"] == true)
}

/// A single-select choice is an index; a multi-select choice is a sorted list of indexes.
fn selected(choice: &Value, index: usize) -> bool {
    let index = Some(index as u64);
    choice.as_u64() == index
        || choice
            .as_array()
            .is_some_and(|picked| picked.iter().any(|n| n.as_u64() == index))
}

fn answered(question: &Value, choices: &Value) -> bool {
    let choice = &choices[question["id"].as_str().unwrap_or("")];
    if question["multiSelect"] == true {
        choice.as_array().is_some_and(|picked| !picked.is_empty())
    } else {
        choice.as_u64().is_some()
    }
}

/// One-based option numbers in the text-reply syntax: `2`, or `1,3` for multi-select.
fn reply(question: &Value, choices: &Value) -> String {
    let choice = &choices[question["id"].as_str().unwrap_or("")];
    match choice.as_array() {
        Some(picked) => picked
            .iter()
            .filter_map(Value::as_u64)
            .map(|n| (n + 1).to_string())
            .collect::<Vec<_>>()
            .join(","),
        None => choice
            .as_u64()
            .map_or(String::new(), |n| (n + 1).to_string()),
    }
}

/// Previews are code or monospace mockups: keep them in a fence, and close one cut by clipping.
fn preview(option: &Value) -> Option<String> {
    let text = option["preview"]
        .as_str()
        .map(str::trim_end)
        .filter(|s| !s.trim().is_empty())?;
    Some(if !text.contains("```") {
        format!("```\n{text}\n```")
    } else if text.matches("```").count() % 2 == 1 {
        format!("{text}\n```")
    } else {
        text.to_owned()
    })
}

fn layout(
    card: &Value,
    request: &Value,
    id: &str,
    choices: &Value,
    done: bool,
    language: &str,
    previews: bool,
) -> Option<Value> {
    let questions = questions(request)?;
    let old = card["body"]["elements"].as_array()?;
    let mut elements = vec![old.first()?.clone()];
    for q in questions {
        let qid = q["id"].as_str()?;
        let hint = if q["multiSelect"] == true {
            crate::localization::translate(language, "（可多选）")
        } else {
            "".into()
        };
        elements.push(json!({"tag":"markdown","content":format!("**{}: {}**{}",qid,content::clip(q["question"].as_str().unwrap_or(""),1600),hint),"text_size":"normal"}));
        for (index, option) in q["options"].as_array()?.iter().enumerate() {
            let label = content::clip(option["label"].as_str()?, 200);
            if previews && let Some(text) = preview(option) {
                elements.push(json!({"tag":"markdown","content":format!("**{}. {}**\n{}",index+1,label,text),"text_size":"normal"}));
            }
            let selected = selected(&choices[qid], index);
            elements.push(json!({"tag":"button","text":{"tag":"plain_text","content":format!("{}{}",if selected{"✓ "}else{""},label)},"type":if selected{"primary"}else{"default"},"width":"fill","size":"medium","disabled":done,"behaviors":[{"type":"callback","value":{"action":"aieyes.answer","outboxId":id,"questionId":qid,"option":index}}]}));
        }
    }
    if manual(questions) {
        elements.push(json!({"tag":"hr"}));
        elements.push(json!({"tag":"button","text":{"tag":"plain_text","content":crate::localization::translate(language,"提交")},"type":"primary","width":"fill","size":"medium","disabled":done,"behaviors":[{"type":"callback","value":{"action":"aieyes.submit","outboxId":id}}]}));
    }
    if old.len() >= 4 {
        elements.extend_from_slice(&old[old.len() - 2..]);
    }
    let mut card = card.clone();
    card["body"]["elements"] = json!(elements);
    (card.to_string().len() <= 23000).then_some(card)
}

/// Buttons matter more than previews: drop the previews before giving up on the card.
fn render(
    card: &Value,
    request: &Value,
    id: &str,
    choices: &Value,
    done: bool,
    language: &str,
) -> Option<Value> {
    [true, false]
        .into_iter()
        .find_map(|previews| layout(card, request, id, choices, done, language, previews))
}

pub(super) fn attach(db: &Connection, request: &Value) -> Result<()> {
    let row:Option<(String,String)>=db.query_row("SELECT o.id,c.payload FROM channel_outbox o JOIN channel_cards c ON c.outbox_id=o.id WHERE o.rowid=last_insert_rowid()",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((id, raw)) = row else {
        return Ok(());
    };
    let card: Value = serde_json::from_str(&raw)?;
    if let Some(card) = render(&card, request, &id, &json!({}), false, &db::language(db)) {
        db.execute(
            "INSERT INTO channel_card_actions(outbox_id,request) VALUES(?1,?2)",
            params![id, request.to_string()],
        )?;
        db.execute(
            "UPDATE channel_cards SET payload=?1 WHERE outbox_id=?2",
            params![card.to_string(), id],
        )?;
    }
    Ok(())
}

fn toast(language: &str, kind: &str, message: &str) -> Value {
    json!({"toast":{"type":kind,"content":crate::localization::translate(language,message)}})
}

pub(super) fn ingest(store: &Store, epoch: &str, event: &Value) -> Result<Value> {
    let _lock = db::config_lock(store)?;
    let c = configuration(&store.db)?;
    let language = db::language(&store.db);
    let stale = || toast(&language, "warning", "选项已失效，请回复最新的问题通知");
    let Some(owner) = &c.owner else {
        return Ok(stale());
    };
    let e = &event["event"];
    let operator = &e["operator"];
    let context = &e["context"];
    if !c.enabled
        || c.epoch != epoch
        || event["header"]["app_id"].as_str() != Some(&c.app_id)
        || event["header"]["event_type"] != "card.action.trigger"
        || e["host"] != "im_message"
        || e["action"]["tag"] != "button"
    {
        return Ok(stale());
    }
    let tenant = operator["tenant_key"]
        .as_str()
        .or_else(|| event["header"]["tenant_key"].as_str())
        .unwrap_or("");
    if operator["open_id"].as_str() != Some(&owner.open_id)
        || context["open_chat_id"].as_str() != Some(&owner.chat_id)
        || tenant != owner.tenant_key
    {
        return Ok(toast(&language, "error", "只能由已配对账号操作此卡片"));
    }
    let stamp = event["header"]["create_time"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let stamp = if stamp >= 100_000_000_000_000 {
        stamp / 1_000_000
    } else if stamp >= 100_000_000_000 {
        stamp / 1000
    } else {
        stamp
    };
    if (now() - stamp).abs() > 300 {
        return Ok(stale());
    }
    let event_id = event["header"]["event_id"].as_str().unwrap_or("");
    let value = &e["action"]["value"];
    let submit = value["action"] == "aieyes.submit";
    if event_id.is_empty()
        || event_id.len() > 256
        || !(submit || value["action"] == "aieyes.answer")
    {
        return Ok(stale());
    }
    let id = value["outboxId"].as_str().unwrap_or("");
    let qid = value["questionId"].as_str().unwrap_or("");
    let index = value["option"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok());
    if !submit && index.is_none() {
        return Ok(stale());
    }
    let tx = store.db.unchecked_transaction()?;
    let inbox_id = format!("button:{event_id}");
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM channel_inbox WHERE id=?1)",
        [&inbox_id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(toast(&language, "info", "已收到选项，请勿重复提交"));
    }
    let row:Option<(String,String,String,String,String,i64,String)>=tx.query_row("SELECT o.session_id,o.expected,a.request,a.choices,c.payload,a.completed,o.sent_id FROM channel_outbox o JOIN channel_card_actions a ON a.outbox_id=o.id JOIN channel_cards c ON c.outbox_id=o.id WHERE o.id=?1 AND o.epoch=?2 AND o.chat_id=?3 AND o.at>?4 AND o.sent_id IS NOT NULL",params![id,c.epoch,owner.chat_id,now()-db::RETENTION],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?;
    let Some((session_id, expected, request, choices, card, completed, message_id)) = row else {
        return Ok(stale());
    };
    if context["open_message_id"].as_str() != Some(&message_id) {
        return Ok(stale());
    }
    if completed != 0 {
        return Ok(toast(&language, "info", "已收到选项，请勿重复提交"));
    }
    let Some(target) = db::session(&tx, &c.epoch, &session_id)? else {
        return Ok(stale());
    };
    if target.state_key != expected || target.session.phase != Phase::NeedsInput {
        return Ok(stale());
    }
    let request: Value = serde_json::from_str(&request)?;
    if request["params"]["threadId"].as_str() != Some(&target.session.native_session)
        || request["params"]["turnId"].as_str() != Some(&target.session.turn_id)
    {
        return Ok(stale());
    }
    let prompt: Option<String> = tx
        .query_row(
            "SELECT value FROM kv WHERE key=?1",
            [format!("channel.prompt:{session_id}")],
            |r| r.get(0),
        )
        .optional()?;
    let prompt: Value = prompt
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or(Value::Null);
    if prompt["expected"].as_str() != Some(&expected)
        || prompt["hash"].as_str() != Some(&codex::fingerprint(&request))
        || prompt["approval"] == true
    {
        return Ok(stale());
    }
    let Some(questions) = questions(&request) else {
        return Ok(stale());
    };
    let manual = manual(questions);
    let mut choices: Value = serde_json::from_str(&choices)?;
    let complete = |choices: &Value| questions.iter().all(|q| answered(q, choices));
    if let Some(index) = index.filter(|_| !submit) {
        let Some(question) = questions.iter().find(|q| {
            q["id"] == qid
                && q["options"]
                    .as_array()
                    .is_some_and(|options| index < options.len())
        }) else {
            return Ok(stale());
        };
        if question["multiSelect"] == true {
            let mut picked: Vec<u64> = choices[qid]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_default();
            match picked.iter().position(|n| *n == index as u64) {
                Some(at) => {
                    picked.remove(at);
                }
                None => picked.push(index as u64),
            }
            picked.sort_unstable();
            choices[qid] = json!(picked);
        } else {
            choices[qid] = json!(index);
        }
    } else if !manual {
        return Ok(stale());
    } else if !complete(&choices) {
        return Ok(toast(&language, "warning", "请先回答所有问题再提交"));
    }
    let done = if manual { submit } else { complete(&choices) };
    if done {
        let count:i64=tx.query_row("SELECT count(*) FROM channel_commands WHERE epoch=?1 AND status IN ('queued','blocked','automation','dispatching','submitted')",[&c.epoch],|r|r.get(0))?;
        if count >= 100 {
            return Ok(stale());
        }
        let body = if questions.len() == 1 {
            reply(&questions[0], &choices)
        } else {
            questions
                .iter()
                .map(|q| format!("{}={}", q["id"].as_str().unwrap_or(""), reply(q, &choices)))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let command_id = format!("c{}", &crate::models::hash(&format!("button:{id}"))[..10]);
        tx.execute("INSERT OR IGNORE INTO channel_commands(id,epoch,session_id,expected,body,reply_to,at,status) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued')",params![command_id,c.epoch,session_id,expected,body,message_id,now()])?;
    }
    tx.execute(
        "UPDATE channel_card_actions SET choices=?1,completed=?2 WHERE outbox_id=?3",
        params![choices.to_string(), done, id],
    )?;
    tx.execute(
        "INSERT INTO channel_inbox(id,epoch,at,payload,processed) VALUES(?1,?2,?3,'{}',1)",
        params![inbox_id, c.epoch, now()],
    )?;
    let card: Value = serde_json::from_str(&card)?;
    let mut response = toast(
        &language,
        "success",
        if done {
            "已收到选项，正在核对原会话"
        } else if manual {
            "已选择，选完后请点击提交"
        } else {
            "已选择，请继续选择其他问题"
        },
    );
    if let Some(card) = render(&card, &request, id, &choices, done, &language) {
        tx.execute(
            "UPDATE channel_cards SET payload=?1 WHERE outbox_id=?2",
            params![card.to_string(), id],
        )?;
        response["card"] = json!({"type":"raw","data":card});
    }
    tx.commit()?;
    crate::runtime::signal(&store.root).wake();
    Ok(response)
}
