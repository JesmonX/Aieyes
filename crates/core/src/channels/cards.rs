//! Presentation is stored alongside the plain-text outbox entry before delivery.
use super::content;
use crate::sessions::Phase;
use serde_json::{Value, json};

pub(super) fn phase_color(phase: Phase) -> &'static str {
    match phase {
        Phase::NeedsInput => "orange",
        Phase::Complete => "green",
        Phase::Failed => "red",
        Phase::Interrupted | Phase::Unknown => "grey",
        _ => "blue",
    }
}

pub(super) fn command_color(status: &str) -> &'static str {
    match status {
        "confirmed" => "green",
        "blocked" | "uncertain" => "orange",
        "cancelled" => "grey",
        _ => "blue",
    }
}

pub(super) fn build(title: &str, color: &str, context: &str, body: &str, footer: &str) -> Value {
    let mut limit = 16000;
    loop {
        let mut elements = vec![
            json!({"tag":"div","text":{"tag":"plain_text","content":content::clip(context,1000),"text_size":"notation","text_color":"grey"}}),
            json!({"tag":"markdown","content":content::clip(body,limit),"text_size":"normal"}),
        ];
        if !footer.is_empty() {
            elements.push(json!({"tag":"hr"}));
            elements.push(json!({"tag":"div","text":{"tag":"plain_text","content":content::clip(footer,1000),"text_size":"notation","text_color":"grey"}}));
        }
        let card = json!({
            "schema":"2.0",
            "config":{"update_multi":true,"summary":{"content":content::clip(&format!("{title} · {context}"),300)}},
            "header":{"title":{"tag":"plain_text","content":content::clip(title,200)},"template":color},
            "body":{"direction":"vertical","padding":"12px","vertical_spacing":"8px","elements":elements}
        });
        // Bound the actual JSON as well as visible text (quotes and backslashes expand).
        if card.to_string().len() <= 24000 {
            return card;
        }
        limit /= 2;
    }
}
