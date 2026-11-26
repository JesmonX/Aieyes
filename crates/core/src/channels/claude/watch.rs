//! Tells the interaction hook helper that its request was settled somewhere else.
//! Claude Code keeps its own dialog open while a `PermissionRequest` hook runs and does not
//! signal the hook when the user answers there, so the helper follows the transcript:
//! the tool call it is holding is settled once its `tool_result` is written. Only appended bytes
//! are read, and nothing is kept but tool call ids. An `Elicitation` hook is not followed: the
//! agent shows its own form only after that hook has returned.
use serde_json::Value;
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// How far back the first scan looks for the tool call that is waiting.
const BACKLOG: u64 = 4 * 1024 * 1024;
/// Bytes consumed per poll; a longer append is finished on the next ones.
const STEP: u64 = 4 * 1024 * 1024;
/// A single record larger than this is skipped instead of buffered.
const MAX_LINE: usize = 16 * 1024 * 1024;

/// Which tool call of the transcript a hook request belongs to.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Subject {
    /// A tool call with exactly this name and input.
    Tool { name: String, input: Value },
    /// Any open call of this tool, for when the agent reports a rewritten input to its hooks.
    Named { name: String },
    /// An `AskUserQuestion` call with this question hash.
    Question { hash: String },
}

impl Subject {
    fn matches(&self, name: &str, input: &Value) -> bool {
        match self {
            Self::Tool { name: n, input: i } => n == name && i == input,
            Self::Named { name: n } => n == name,
            Self::Question { hash } => {
                name == "AskUserQuestion" && super::question_hash(input) == *hash
            }
        }
    }
}

pub(super) struct Watch {
    path: PathBuf,
    subject: Subject,
    offset: u64,
    partial: Vec<u8>,
    skipping: bool,
    /// Past the backlog: records from here on were written after the request was registered.
    live: bool,
    /// Tool calls matching the subject, in transcript order.
    candidates: Vec<String>,
    resolved: HashSet<String>,
    prompted: bool,
}

impl Watch {
    /// Scans the end of the transcript for the waiting call. `None` when it cannot be read.
    pub fn open(path: &Path, subject: Subject) -> Option<Self> {
        let len = std::fs::metadata(path).ok()?.len();
        let mut watch = Self {
            path: path.to_owned(),
            subject,
            offset: len.saturating_sub(BACKLOG),
            partial: Vec::new(),
            // A scan that starts mid-file starts mid-record.
            skipping: len > BACKLOG,
            live: false,
            candidates: Vec::new(),
            resolved: HashSet::new(),
            prompted: false,
        };
        watch.read(len);
        // Calls that already have their result are earlier ones, not the one that is waiting.
        let resolved = std::mem::take(&mut watch.resolved);
        watch.candidates.retain(|id| !resolved.contains(id));
        watch.live = true;
        Some(watch)
    }

    /// Like `open`, falling back from an exact tool call to any open call of the same tool.
    pub fn open_tool(path: &Path, name: &str, input: &Value) -> Option<Self> {
        let exact = Self::open(
            path,
            Subject::Tool {
                name: name.to_owned(),
                input: input.clone(),
            },
        )?;
        if !exact.candidates.is_empty() {
            return Some(exact);
        }
        Self::open(
            path,
            Subject::Named {
                name: name.to_owned(),
            },
        )
    }

    /// The id of the waiting tool call, when exactly one call matches.
    pub fn call_id(&self) -> Option<&str> {
        let mut open = self
            .candidates
            .iter()
            .filter(|id| !self.resolved.contains(*id));
        match (open.next(), open.next()) {
            (Some(id), None) => Some(id),
            _ => None,
        }
    }

    /// Reads what was appended and reports whether the request is settled: every matching call
    /// has its result, or the user has moved on to a new prompt.
    pub fn settled(&mut self) -> bool {
        let Ok(len) = std::fs::metadata(&self.path).map(|m| m.len()) else {
            return false;
        };
        if len < self.offset {
            // Rewritten (compaction or rewind): the positions no longer mean anything.
            return true;
        }
        self.read(len.min(self.offset + STEP));
        self.prompted
            || (!self.candidates.is_empty()
                && self.candidates.iter().all(|id| self.resolved.contains(id)))
    }

    fn read(&mut self, until: u64) {
        if until <= self.offset {
            return;
        }
        let Ok(mut file) = File::open(&self.path) else {
            return;
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if file
            .take(until - self.offset)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return;
        }
        self.offset += bytes.len() as u64;
        for chunk in bytes.split_inclusive(|b| *b == b'\n') {
            let complete = chunk.last() == Some(&b'\n');
            if self.skipping {
                self.skipping = !complete;
                continue;
            }
            if self.partial.len() + chunk.len() > MAX_LINE {
                self.partial.clear();
                self.skipping = !complete;
                continue;
            }
            self.partial.extend_from_slice(chunk);
            if complete {
                let line = std::mem::take(&mut self.partial);
                if let Ok(record) = serde_json::from_slice::<Value>(&line) {
                    self.record(&record);
                }
            }
        }
    }

    fn record(&mut self, record: &Value) {
        // Subagent turns have their own requests; they say nothing about this one.
        if record["isSidechain"] == true {
            return;
        }
        let content = &record["message"]["content"];
        match record["type"].as_str() {
            Some("assistant") => {
                for block in content.as_array().into_iter().flatten() {
                    if block["type"] == "tool_use"
                        && let Some(id) = block["id"].as_str().filter(|s| !s.is_empty())
                        && self
                            .subject
                            .matches(block["name"].as_str().unwrap_or(""), &block["input"])
                        && !self.candidates.iter().any(|c| c == id)
                    {
                        self.candidates.push(id.to_owned());
                    }
                }
            }
            Some("user") => {
                let mut results = false;
                for block in content.as_array().into_iter().flatten() {
                    if block["type"] == "tool_result" {
                        results = true;
                        if let Some(id) = block["tool_use_id"].as_str() {
                            self.resolved.insert(id.to_owned());
                        }
                    }
                }
                // A typed prompt after the request was registered: the user answered or
                // interrupted it in the terminal and went on.
                if self.live && !results && record["isMeta"] != true {
                    let typed = content.as_str().is_some_and(|s| !s.trim().is_empty())
                        || content
                            .as_array()
                            .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "text"));
                    self.prompted |= typed;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn append(path: &Path, rows: &[Value]) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
    }

    fn tool_use(id: &str, name: &str, input: Value) -> Value {
        json!({"type":"assistant","message":{"id":"msg","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}})
    }

    fn tool_result(id: &str) -> Value {
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"ok"}]}})
    }

    #[test]
    fn a_request_is_settled_by_the_result_of_its_own_tool_call_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let input = json!({"command":"git push","description":"Push"});
        append(
            &path,
            &[
                json!({"type":"user","message":{"content":"push it"}}),
                // An earlier identical call that already ran is not the waiting one.
                tool_use("old", "Bash", input.clone()),
                tool_result("old"),
                tool_use("other", "Bash", json!({"command":"ls"})),
                tool_use("mine", "Bash", input.clone()),
            ],
        );
        let mut watch = Watch::open(
            &path,
            Subject::Tool {
                name: "Bash".into(),
                input,
            },
        )
        .unwrap();
        assert_eq!(watch.call_id(), Some("mine"));
        assert!(!watch.settled());
        // An input the agent rewrote for its hooks still finds the open call of that tool.
        let rewritten = Watch::open_tool(&path, "Bash", &json!({"command": "git push "})).unwrap();
        assert_eq!(rewritten.candidates, ["other", "mine"]);
        assert_eq!(rewritten.call_id(), None);
        let exact = Watch::open_tool(&path, "Bash", &json!({"command": "ls"})).unwrap();
        assert_eq!(exact.call_id(), Some("other"));
        // A parallel call finishing, a subagent prompt and a half-written record change nothing.
        append(
            &path,
            &[
                tool_result("other"),
                json!({"type":"user","isSidechain":true,"message":{"content":"subagent task"}}),
            ],
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(
                br#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"mi"#,
            )
            .unwrap();
        assert!(!watch.settled());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"ne\"}]}}\n")
            .unwrap();
        assert!(watch.settled());
    }

    #[test]
    fn unknown_calls_settle_on_a_new_prompt_and_rewrites_always_settle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        append(&path, &[json!({"type":"user","message":{"content":"go"}})]);
        let subject = Subject::Question { hash: "h".into() };
        let mut watch = Watch::open(&path, subject.clone()).unwrap();
        assert_eq!(watch.call_id(), None);
        assert!(!watch.settled());
        append(
            &path,
            &[
                tool_result("unrelated"),
                json!({"type":"user","isMeta":true,"message":{"content":"<caveat>"}}),
            ],
        );
        assert!(!watch.settled());
        append(
            &path,
            &[json!({"type":"user","message":{"content":[{"type":"text","text":"never mind"}]}})],
        );
        assert!(watch.settled());

        let mut watch = Watch::open(&path, subject).unwrap();
        assert!(!watch.settled());
        std::fs::write(&path, b"").unwrap();
        assert!(watch.settled());
        assert!(
            Watch::open(
                &dir.path().join("missing.jsonl"),
                Subject::Question { hash: "h".into() }
            )
            .is_none()
        );
    }

    #[test]
    fn questions_and_mcp_servers_are_matched_by_hash_and_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let questions = json!({"questions":[{"question":"Which?","header":"Pick","options":[{"label":"A"}],"multiSelect":false}]});
        append(
            &path,
            &[
                tool_use("q1", "AskUserQuestion", questions.clone()),
                tool_use("m1", "mcp__my_server__deploy", json!({"env":"prod"})),
                tool_use("m2", "mcp__other__deploy", json!({})),
            ],
        );
        let mut asked = Watch::open(
            &path,
            Subject::Question {
                hash: crate::channels::claude::question_hash(&questions),
            },
        )
        .unwrap();
        assert_eq!(asked.call_id(), Some("q1"));
        append(&path, &[tool_result("m2")]);
        assert!(!asked.settled());
        append(&path, &[tool_result("m1"), tool_result("q1")]);
        assert!(asked.settled());
    }
}
