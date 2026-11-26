//! Bounded streaming JSONL line reader and metadata projector.
//!
//! Normal lines (<= `FAST_LINE_LIMIT`) take the fast path (`serde_json::from_slice`).
//! Oversized lines (e.g. >16 MiB Codex `response_item` base64 images/tool outputs or
//! large Claude/Pi/CodeBuddy/Harness messages) are streamed in O(1) memory without
//! requiring `Seek`, validating RFC 8259 JSON and UTF-8 while projecting only the
//! lightweight metering/lifecycle fields Aieyes needs.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read};

/// Lines up to this size are buffered whole and parsed directly via `serde_json`.
pub(crate) const FAST_LINE_LIMIT: usize = 2 * 1024 * 1024;
/// Unterminated non-JSON tails above this size report the legacy 16 MiB line error.
pub(crate) const MAX_RAW_UNTERMINATED: u64 = 16 * 1024 * 1024;
/// Absolute physical safety cap for a single streamed JSONL line.
pub(crate) const HARD_LINE_LIMIT: u64 = 512 * 1024 * 1024;
/// Maximum size of the projected JSON buffer emitted on the slow path.
const MAX_PRUNED_BYTES: usize = 1024 * 1024;
/// Maximum raw JSON content bytes kept for any single string value on the slow path.
const MAX_KEPT_STRING_BYTES: usize = 16 * 1024;
/// Maximum raw JSON key length inspected on the slow path; longer keys are skipped.
const MAX_KEY_BYTES: usize = 512;
/// Maximum nesting depth for emitted structures.
const MAX_DEPTH: usize = 128;
/// Maximum nesting depth allowed inside skipped subtrees.
const MAX_SKIP_DEPTH: usize = 4096;
/// Maximum elements emitted for a kept array on the slow path.
const MAX_KEPT_ARRAY_ITEMS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Usage / metering / attribution / inherited history.
    History,
    /// Live session lifecycle events.
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineError {
    InvalidJson,
    MetadataTooLarge,
}

#[derive(Debug)]
pub(crate) enum LineRecord {
    /// Reached EOF cleanly with 0 bytes consumed.
    Eof,
    /// Reached EOF (or `max_remaining`) before `\n`; caller retains cursor position.
    Incomplete { bytes: u64 },
    /// Exceeded hard line limit or >16 MiB unterminated non-JSON tail.
    Oversized,
    /// Consumed a complete `\n`-terminated line of `bytes` raw bytes.
    Complete {
        bytes: u64,
        parsed: std::result::Result<Value, LineError>,
    },
}

/// Read and parse one JSONL line from any `BufRead` (including non-seekable zstd streams).
pub(crate) fn read_line<R: BufRead>(
    reader: &mut R,
    mode: Mode,
    provider: &str,
    max_remaining: Option<u64>,
    hasher: Option<&mut Sha256>,
) -> std::io::Result<LineRecord> {
    read_line_with_limits(
        reader,
        mode,
        provider,
        max_remaining,
        hasher,
        FAST_LINE_LIMIT,
        MAX_RAW_UNTERMINATED,
        HARD_LINE_LIMIT,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_line_with_limits<R: BufRead>(
    reader: &mut R,
    mode: Mode,
    provider: &str,
    max_remaining: Option<u64>,
    mut hasher: Option<&mut Sha256>,
    fast_limit: usize,
    max_raw_unterminated: u64,
    hard_limit: u64,
) -> std::io::Result<LineRecord> {
    let cap = max_remaining.map_or(hard_limit, |m| m.min(hard_limit));
    let fast_take = (fast_limit as u64).min(cap);
    let probe_take = if fast_take < cap {
        fast_take + 1
    } else {
        fast_take
    };
    let mut buf = Vec::new();
    let n = reader
        .by_ref()
        .take(probe_take)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(LineRecord::Eof);
    }
    let ended = buf.last() == Some(&b'\n');
    if ended && buf.len() <= fast_limit {
        if let Some(h) = hasher.as_deref_mut() {
            h.update(&buf);
        }
        let parsed = serde_json::from_slice::<Value>(&buf).map_err(|_| LineError::InvalidJson);
        return Ok(LineRecord::Complete {
            bytes: buf.len() as u64,
            parsed,
        });
    }
    if !ended && (buf.len() as u64) < probe_take {
        // Reached true EOF within the fast limit without a newline.
        return Ok(LineRecord::Incomplete {
            bytes: buf.len() as u64,
        });
    }
    if !ended && max_remaining.is_some_and(|m| buf.len() as u64 >= m) && buf.len() <= fast_limit {
        // Reached `max_remaining` boundary before `\n` (e.g. codex_history boundaryInvalid).
        return Ok(LineRecord::Incomplete {
            bytes: buf.len() as u64,
        });
    }

    // Slow path: line exceeds `fast_limit` (or hits boundary). Stream-project in O(1) memory.
    let mut stream = LineStream::new(&buf, reader, hasher, cap);
    let mut pruner = Pruner::new(mode, provider);
    let outcome = pruner.parse_line(&mut stream)?;
    stream.drain_to_newline()?;

    let bytes = stream.bytes_read;
    if stream.hit_limit {
        // If capped by caller's `max_remaining` (strictly below `hard_limit`), signal Incomplete
        // so callers like `codex_history` report `boundaryInvalid` rather than a generic size error.
        if max_remaining.is_some_and(|m| m < hard_limit && bytes >= m) {
            return Ok(LineRecord::Incomplete { bytes });
        }
        return Ok(LineRecord::Oversized);
    }

    if !stream.saw_newline {
        // Reached EOF without `\n`.
        let valid_in_flight =
            matches!(outcome, Ok(()) | Err(PruneErr::Eof)) && pruner.root_is_object;
        if bytes > max_raw_unterminated && !valid_in_flight {
            return Ok(LineRecord::Oversized);
        }
        return Ok(LineRecord::Incomplete { bytes });
    }

    let parsed = match outcome {
        Ok(()) => {
            if pruner.root_is_object {
                serde_json::from_slice::<Value>(&pruner.out).map_err(|_| LineError::InvalidJson)
            } else {
                // Non-object valid JSON (array/primitive) on an oversized line carries no record fields.
                Ok(Value::Null)
            }
        }
        Err(PruneErr::MetadataTooLarge) => Err(LineError::MetadataTooLarge),
        Err(PruneErr::Syntax | PruneErr::Eof | PruneErr::Depth) => Err(LineError::InvalidJson),
        Err(PruneErr::Limit) => return Ok(LineRecord::Oversized),
    };
    Ok(LineRecord::Complete { bytes, parsed })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Filter,
    KeepAll,
    Skip,
}

fn field_action(mode: Mode, provider: &str, path: &[String], key: &str) -> Action {
    let p: Vec<&str> = path.iter().map(String::as_str).collect();
    match mode {
        Mode::History => history_action(provider, &p, key),
        Mode::Session => session_action(provider, &p, key),
    }
}

fn is_normalized_root(key: &str) -> bool {
    matches!(
        key,
        "id" | "sessionId"
            | "timestamp"
            | "timestampMs"
            | "time"
            | "model"
            | "tokens"
            | "billing"
            | "intervalStart"
            | "intervalEvidence"
            | "requestTiming"
            | "inheritance"
            | "nativeCredits"
            | "clineTaskUsageVersion"
    )
}

fn history_action(provider: &str, path: &[&str], key: &str) -> Action {
    match provider {
        "codex" => match path {
            [] => match key {
                "payload" => Action::Filter,
                "type" | "timestamp" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["payload"] => match key {
                "item" | "info" => Action::Filter,
                "type"
                | "turn_id"
                | "id"
                | "session_id"
                | "timestamp"
                | "model_provider"
                | "originator"
                | "creator_user_id"
                | "creator_account_id"
                | "creator_identity_key"
                | "history_base"
                | "model"
                | "role"
                | "completed_at_ms"
                | "rate_limits" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["payload", "item"] => match key {
                "type" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["payload", "info"] => match key {
                "total_token_usage" | "last_token_usage" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "claude" | "qoder" | "qoder-cn" => match path {
            [] => match key {
                "message" => Action::Filter,
                "type" | "isApiErrorMessage" | "isSidechain" | "modelSource" | "uuid"
                | "requestId" | "requestRef" => Action::KeepAll,
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message"] => match key {
                "id" | "model" | "usage" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "pi" | "omp" => match path {
            [] => match key {
                "message" => Action::Filter,
                "type" | "parentId" | "usage" | "provider" | "stopReason" => Action::KeepAll,
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message"] => match key {
                "role" | "model" | "provider" | "timestamp" | "stopReason" | "usage" => {
                    Action::KeepAll
                }
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "kimi" => match path {
            [] => match key {
                "message" => Action::Filter,
                "type" | "turnId" | "agentId" | "modelAlias" | "provider" | "usage" => {
                    Action::KeepAll
                }
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message"] => match key {
                "payload" => Action::Filter,
                "type" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message", "payload"] => match key {
                "message_id" | "token_usage" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "codebuddy" => match path {
            [] => match key {
                "providerData" => Action::Filter,
                "type" | "role" | "status" => Action::KeepAll,
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["providerData"] => match key {
                "model"
                | "requestModelId"
                | "messageId"
                | "conversationRequestId"
                | "requestId"
                | "rawUsage"
                | "usage"
                | "credit"
                | "source"
                | "stageId" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "copilot" => match path {
            [] => match key {
                "data" => Action::Filter,
                "type" => Action::KeepAll,
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data"] => match key {
                "usage" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "grok" => match path {
            [] => match key {
                "params" => Action::Filter,
                "method" => Action::KeepAll,
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["params"] => match key {
                "update" => Action::Filter,
                "sessionId" | "_meta" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["params", "update"] => match key {
                "usage" => Action::Filter,
                "sessionUpdate" | "prompt_id" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["params", "update", "usage"] => match key {
                "modelUsage" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "deepseek-harness" => match path {
            [] => match key {
                "data" => Action::Filter,
                "type" | "version" | "seedLength" | "isSeeded" | "seq" => Action::KeepAll,
                k if is_normalized_root(k) => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data"] => match key {
                "message" => Action::Filter,
                "turn" | "step" | "usage" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data", "message"] => match key {
                "source" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        _ => match path {
            [] if is_normalized_root(key) => Action::KeepAll,
            _ => Action::Skip,
        },
    }
}

fn session_action(provider: &str, path: &[&str], key: &str) -> Action {
    match provider {
        "hook" => match path {
            [] => match key {
                "kind" | "session" | "sessionSource" | "classification" | "originator"
                | "modelProvider" | "parentSession" | "parentSessionPath" | "turn" | "record"
                | "parent" | "call" | "tool" | "at" | "pid" | "processStart" | "fullyIdle"
                | "failed" | "interrupted" | "notification" | "questionAccepted"
                | "answeredCalls" | "workspace" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "codex" => match path {
            [] => match key {
                "payload" => Action::Filter,
                "type" | "timestamp" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["payload"] => match key {
                "source" | "content" => Action::Filter,
                "type"
                | "id"
                | "session_id"
                | "originator"
                | "model_provider"
                | "turn_id"
                | "internal_chat_message_metadata_passthrough"
                | "call_id"
                | "name"
                | "role"
                | "output"
                | "history_base" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["payload", "source"] => match key {
                "subagent" => Action::Filter,
                _ => Action::Skip,
            },
            ["payload", "source", "subagent"] => match key {
                "thread_spawn" => Action::Filter,
                _ => Action::Skip,
            },
            ["payload", "source", "subagent", "thread_spawn"] => match key {
                "parent_thread_id" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["payload", "content", "*"] => match key {
                "text" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "claude" | "qoder" | "qoder-cn" => match path {
            [] => match key {
                "message" => Action::Filter,
                "type" | "timestamp" | "sessionId" | "entrypoint" | "parentSessionId"
                | "parent_session_id" | "isSidechain" | "agentId" | "isMeta"
                | "isApiErrorMessage" | "uuid" | "parentUuid" | "subtype" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message"] => match key {
                "content" => Action::Filter,
                "stop_reason" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message", "content", "*"] => match key {
                "type" | "name" | "id" | "tool_use_id" | "text" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "pi" | "omp" => match path {
            [] => match key {
                "data" | "message" => Action::Filter,
                "type" | "timestamp" | "id" | "parentId" | "parentSessionId"
                | "parent_session_id" | "parentSession" | "customType" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data"] => match key {
                "pendingToolCalls" => Action::Filter,
                "toolCallId" | "toolName" | "kind" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message"] => match key {
                "content" => Action::Filter,
                "role" | "stopReason" | "toolCallId" | "toolName" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message", "content", "*"] => match key {
                "type" | "id" | "name" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "kimi" => match path {
            [] => match key {
                "event" | "message" => Action::Filter,
                "type" | "time" | "timestamp" | "turnId" | "id" | "interactionId" | "toolName"
                | "kind" | "reason" | "parentSessionId" | "parent_session_id" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["event"] => match key {
                "type" | "toolCallId" | "tool_call_id" | "callId" | "call_id" | "id" | "name"
                | "tool_name" | "toolName" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["message"] => match key {
                "type" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "grok" => match path {
            [] => match key {
                "type" | "ts" | "session_id" | "turn_number" | "phase" | "outcome"
                | "permission_id" | "request_id" | "tool_call_id" | "call_id" | "id"
                | "tool_name" | "name" | "parentSessionId" | "parent_session_id" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "codebuddy" => match path {
            [] => match key {
                "providerData" => Action::Filter,
                "type" | "role" | "status" | "timestamp" | "sessionId" | "id" | "parentId"
                | "isSidechain" | "agentId" | "callId" | "call_id" | "toolCallId"
                | "tool_call_id" | "name" | "tool_name" | "toolName" | "parentSessionId"
                | "parent_session_id" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["providerData"] => match key {
                "error" => Action::Filter,
                "conversationRequestId" | "skipRun" | "messageId" | "rawUsage" | "usage" => {
                    Action::KeepAll
                }
                _ => Action::Skip,
            },
            ["providerData", "error"] => match key {
                "message" | "isRetryable" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "copilot" => match path {
            [] => match key {
                "data" => Action::Filter,
                "type" | "timestamp" | "parentSessionId" | "parent_session_id" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data"] => match key {
                "toolRequests" => Action::Filter,
                "interactionId" | "sessionId" | "requestId" | "toolCallId" | "tool_call_id"
                | "callId" | "call_id" | "id" | "toolName" | "tool_name" | "name" | "content" => {
                    Action::KeepAll
                }
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        "deepseek-harness" => match path {
            [] => match key {
                "data" => Action::Filter,
                "type" | "version" | "id" | "seedLength" | "isSeeded" | "seq" | "time"
                | "parentSessionId" | "parent_session_id" | "_aieyesInherited" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data"] => match key {
                "reason" | "message" => Action::Filter,
                "turn" | "id" | "callId" | "call_id" | "toolCallId" | "tool_call_id" | "name"
                | "tool_name" | "toolName" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data", "reason"] => match key {
                "kind" => Action::KeepAll,
                _ => Action::Skip,
            },
            ["data", "message"] => match key {
                "toolCallId" | "tool_call_id" | "callId" | "call_id" | "id" | "name"
                | "tool_name" | "toolName" => Action::KeepAll,
                _ => Action::Skip,
            },
            _ => Action::Skip,
        },
        _ => session_action("claude", path, key),
    }
}

/// Streams a single line (already-read `prefix` + remaining bytes in `reader` up to `\n`)
/// without ever reading past `\n` and without requiring `Seek`.
struct LineStream<'a, R: BufRead> {
    prefix: &'a [u8],
    prefix_pos: usize,
    chunk: Vec<u8>,
    chunk_pos: usize,
    reader: &'a mut R,
    hasher: Option<&'a mut Sha256>,
    bytes_read: u64,
    max_bytes: u64,
    saw_newline: bool,
    hit_limit: bool,
    peeked: Option<u8>,
}

impl<'a, R: BufRead> LineStream<'a, R> {
    fn new(
        prefix: &'a [u8],
        reader: &'a mut R,
        mut hasher: Option<&'a mut Sha256>,
        max_bytes: u64,
    ) -> Self {
        if let Some(h) = hasher.as_deref_mut() {
            h.update(prefix);
        }
        let saw_newline = prefix.last() == Some(&b'\n');
        Self {
            prefix,
            prefix_pos: 0,
            chunk: Vec::new(),
            chunk_pos: 0,
            reader,
            hasher,
            bytes_read: prefix.len() as u64,
            max_bytes,
            saw_newline,
            hit_limit: false,
            peeked: None,
        }
    }

    fn refill(&mut self) -> std::io::Result<bool> {
        if self.saw_newline || self.hit_limit {
            return Ok(false);
        }
        let buf = self.reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(false);
        }
        let remaining = self.max_bytes.saturating_sub(self.bytes_read);
        if remaining == 0 {
            self.hit_limit = true;
            return Ok(false);
        }
        let cap = buf.len().min(remaining as usize).min(64 * 1024);
        let slice = &buf[..cap];
        let take = match slice.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                self.saw_newline = true;
                idx + 1
            }
            None => cap,
        };
        self.chunk.clear();
        self.chunk.extend_from_slice(&slice[..take]);
        self.chunk_pos = 0;
        self.reader.consume(take);
        self.bytes_read += take as u64;
        if let Some(h) = self.hasher.as_deref_mut() {
            h.update(&self.chunk);
        }
        Ok(true)
    }

    fn next_byte(&mut self) -> std::io::Result<Option<u8>> {
        if let Some(b) = self.peeked.take() {
            return Ok(Some(b));
        }
        if self.prefix_pos < self.prefix.len() {
            let b = self.prefix[self.prefix_pos];
            self.prefix_pos += 1;
            return Ok(Some(b));
        }
        if self.chunk_pos < self.chunk.len() {
            let b = self.chunk[self.chunk_pos];
            self.chunk_pos += 1;
            return Ok(Some(b));
        }
        if self.refill()? {
            let b = self.chunk[self.chunk_pos];
            self.chunk_pos += 1;
            return Ok(Some(b));
        }
        Ok(None)
    }

    fn peek_byte(&mut self) -> std::io::Result<Option<u8>> {
        if self.peeked.is_none() {
            self.peeked = self.next_byte()?;
        }
        Ok(self.peeked)
    }

    fn drain_to_newline(&mut self) -> std::io::Result<()> {
        self.peeked = None;
        self.prefix_pos = self.prefix.len();
        self.chunk_pos = self.chunk.len();
        while !self.saw_newline && !self.hit_limit {
            if !self.refill()? {
                break;
            }
            self.chunk_pos = self.chunk.len();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PruneErr {
    Syntax,
    Eof,
    Depth,
    MetadataTooLarge,
    Limit,
}

struct Pruner<'a> {
    mode: Mode,
    provider: &'a str,
    out: Vec<u8>,
    path: Vec<String>,
    root_is_object: bool,
}

impl<'a> Pruner<'a> {
    fn new(mode: Mode, provider: &'a str) -> Self {
        Self {
            mode,
            provider,
            out: Vec::with_capacity(4096),
            path: Vec::new(),
            root_is_object: false,
        }
    }

    fn push_byte(&mut self, b: u8) -> Result<(), PruneErr> {
        if self.out.len() + 1 > MAX_PRUNED_BYTES {
            return Err(PruneErr::MetadataTooLarge);
        }
        self.out.push(b);
        Ok(())
    }

    fn push_slice(&mut self, s: &[u8]) -> Result<(), PruneErr> {
        if self.out.len() + s.len() > MAX_PRUNED_BYTES {
            return Err(PruneErr::MetadataTooLarge);
        }
        self.out.extend_from_slice(s);
        Ok(())
    }

    fn parse_line<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
    ) -> std::io::Result<Result<(), PruneErr>> {
        let Some(first) = self.skip_ws(stream)? else {
            return Ok(Err(if stream.hit_limit {
                PruneErr::Limit
            } else {
                PruneErr::Eof
            }));
        };
        if first == b'{' {
            self.root_is_object = true;
            if let Err(err) = self.parse_value(stream, Action::Filter)? {
                return Ok(Err(err));
            }
        } else {
            if let Err(err) = self.skip_value(stream)? {
                return Ok(Err(err));
            }
        }
        match self.skip_ws(stream)? {
            None => Ok(if stream.hit_limit {
                Err(PruneErr::Limit)
            } else {
                Ok(())
            }),
            Some(_) => Ok(Err(PruneErr::Syntax)),
        }
    }

    fn skip_ws<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
    ) -> std::io::Result<Option<u8>> {
        while let Some(b) = stream.peek_byte()? {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                stream.next_byte()?;
            } else {
                return Ok(Some(b));
            }
        }
        Ok(None)
    }

    fn next_required<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
    ) -> std::io::Result<Result<u8, PruneErr>> {
        match stream.next_byte()? {
            Some(b) => Ok(Ok(b)),
            None => Ok(Err(if stream.hit_limit {
                PruneErr::Limit
            } else {
                PruneErr::Eof
            })),
        }
    }

    /// Iterative RFC 8259 parser for kept (`Filter` / `KeepAll`) values.
    fn parse_value<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
        root_action: Action,
    ) -> std::io::Result<Result<(), PruneErr>> {
        enum Container {
            Object {
                action: Action,
                has_emitted: bool,
                pushed_path: bool,
                expect_first_or_end: bool,
            },
            Array {
                action: Action,
                emitted_count: usize,
                pushed_path: bool,
                expect_first_or_end: bool,
            },
        }

        let mut stack: Vec<Container> = Vec::new();
        let mut next_action = root_action;
        let mut next_pushed_path = false;

        'value: loop {
            let Some(b) = self.skip_ws(stream)? else {
                return Ok(Err(if stream.hit_limit {
                    PruneErr::Limit
                } else {
                    PruneErr::Eof
                }));
            };
            match b {
                b'{' => {
                    stream.next_byte()?;
                    if stack.len() >= MAX_DEPTH {
                        return Ok(Err(PruneErr::Depth));
                    }
                    if let Err(e) = self.push_byte(b'{') {
                        return Ok(Err(e));
                    }
                    stack.push(Container::Object {
                        action: next_action,
                        has_emitted: false,
                        pushed_path: next_pushed_path,
                        expect_first_or_end: true,
                    });
                }
                b'[' => {
                    stream.next_byte()?;
                    if stack.len() >= MAX_DEPTH {
                        return Ok(Err(PruneErr::Depth));
                    }
                    if let Err(e) = self.push_byte(b'[') {
                        return Ok(Err(e));
                    }
                    stack.push(Container::Array {
                        action: next_action,
                        emitted_count: 0,
                        pushed_path: next_pushed_path,
                        expect_first_or_end: true,
                    });
                }
                b'"' => {
                    if let Err(e) = self.parse_string(stream, Some(MAX_KEPT_STRING_BYTES))? {
                        return Ok(Err(e));
                    }
                    if next_pushed_path {
                        self.path.pop();
                    }
                }
                b'-' | b'0'..=b'9' => {
                    if let Err(e) = self.parse_number(stream, true)? {
                        return Ok(Err(e));
                    }
                    if next_pushed_path {
                        self.path.pop();
                    }
                }
                b't' => {
                    if let Err(e) = self.parse_literal(stream, b"true", true)? {
                        return Ok(Err(e));
                    }
                    if next_pushed_path {
                        self.path.pop();
                    }
                }
                b'f' => {
                    if let Err(e) = self.parse_literal(stream, b"false", true)? {
                        return Ok(Err(e));
                    }
                    if next_pushed_path {
                        self.path.pop();
                    }
                }
                b'n' => {
                    if let Err(e) = self.parse_literal(stream, b"null", true)? {
                        return Ok(Err(e));
                    }
                    if next_pushed_path {
                        self.path.pop();
                    }
                }
                _ => return Ok(Err(PruneErr::Syntax)),
            }

            // Advance parent containers until we either descend into the next value or finish.
            loop {
                let Some(top) = stack.last_mut() else {
                    return Ok(Ok(()));
                };
                match top {
                    Container::Object {
                        action,
                        has_emitted,
                        pushed_path,
                        expect_first_or_end,
                    } => {
                        let parent_action = *action;
                        let first = *expect_first_or_end;
                        let Some(next) = self.skip_ws(stream)? else {
                            return Ok(Err(if stream.hit_limit {
                                PruneErr::Limit
                            } else {
                                PruneErr::Eof
                            }));
                        };
                        if next == b'}' {
                            stream.next_byte()?;
                            if let Err(e) = self.push_byte(b'}') {
                                return Ok(Err(e));
                            }
                            let pop_path = *pushed_path;
                            stack.pop();
                            if pop_path {
                                self.path.pop();
                            }
                            continue;
                        }
                        if !first {
                            if next != b',' {
                                return Ok(Err(PruneErr::Syntax));
                            }
                            stream.next_byte()?;
                            let Some(after_comma) = self.skip_ws(stream)? else {
                                return Ok(Err(if stream.hit_limit {
                                    PruneErr::Limit
                                } else {
                                    PruneErr::Eof
                                }));
                            };
                            if after_comma != b'"' {
                                return Ok(Err(PruneErr::Syntax));
                            }
                        } else if next != b'"' {
                            return Ok(Err(PruneErr::Syntax));
                        }
                        *expect_first_or_end = false;

                        let key_raw = match self.read_key(stream)? {
                            Ok(k) => k,
                            Err(e) => return Ok(Err(e)),
                        };
                        let Some(colon) = self.skip_ws(stream)? else {
                            return Ok(Err(if stream.hit_limit {
                                PruneErr::Limit
                            } else {
                                PruneErr::Eof
                            }));
                        };
                        if colon != b':' {
                            return Ok(Err(PruneErr::Syntax));
                        }
                        stream.next_byte()?;

                        let child_decision = match (parent_action, key_raw.as_ref()) {
                            (Action::KeepAll, Some(_)) => (Action::KeepAll, None),
                            (Action::KeepAll, None) => (Action::Skip, None),
                            (Action::Filter, Some(raw)) => {
                                match serde_json::from_slice::<String>(raw) {
                                    Ok(decoded) => {
                                        let act = field_action(
                                            self.mode,
                                            self.provider,
                                            &self.path,
                                            &decoded,
                                        );
                                        (act, Some(decoded))
                                    }
                                    Err(_) => return Ok(Err(PruneErr::Syntax)),
                                }
                            }
                            _ => (Action::Skip, None),
                        };

                        match child_decision {
                            (Action::Skip, _) => {
                                if let Err(e) = self.skip_value(stream)? {
                                    return Ok(Err(e));
                                }
                                continue;
                            }
                            (act, decoded_key) => {
                                if *has_emitted && let Err(e) = self.push_byte(b',') {
                                    return Ok(Err(e));
                                }
                                *has_emitted = true;
                                if let Some(raw) = key_raw.as_ref()
                                    && let Err(e) = self.push_slice(raw)
                                {
                                    return Ok(Err(e));
                                }
                                if let Err(e) = self.push_byte(b':') {
                                    return Ok(Err(e));
                                }
                                if act == Action::Filter
                                    && let Some(k) = decoded_key
                                {
                                    self.path.push(k);
                                    next_pushed_path = true;
                                } else {
                                    next_pushed_path = false;
                                }
                                next_action = act;
                                continue 'value;
                            }
                        }
                    }
                    Container::Array {
                        action,
                        emitted_count,
                        pushed_path,
                        expect_first_or_end,
                    } => {
                        let parent_action = *action;
                        let first = *expect_first_or_end;
                        let Some(next) = self.skip_ws(stream)? else {
                            return Ok(Err(if stream.hit_limit {
                                PruneErr::Limit
                            } else {
                                PruneErr::Eof
                            }));
                        };
                        if next == b']' {
                            stream.next_byte()?;
                            if let Err(e) = self.push_byte(b']') {
                                return Ok(Err(e));
                            }
                            let pop_path = *pushed_path;
                            stack.pop();
                            if pop_path {
                                self.path.pop();
                            }
                            continue;
                        }
                        if !first {
                            if next != b',' {
                                return Ok(Err(PruneErr::Syntax));
                            }
                            stream.next_byte()?;
                            let Some(after_comma) = self.skip_ws(stream)? else {
                                return Ok(Err(if stream.hit_limit {
                                    PruneErr::Limit
                                } else {
                                    PruneErr::Eof
                                }));
                            };
                            if after_comma == b']' {
                                return Ok(Err(PruneErr::Syntax));
                            }
                        }
                        *expect_first_or_end = false;

                        if *emitted_count >= MAX_KEPT_ARRAY_ITEMS {
                            if let Err(e) = self.skip_value(stream)? {
                                return Ok(Err(e));
                            }
                            continue;
                        }

                        if *emitted_count > 0
                            && let Err(e) = self.push_byte(b',')
                        {
                            return Ok(Err(e));
                        }
                        *emitted_count += 1;
                        if parent_action == Action::Filter {
                            self.path.push("*".into());
                            next_pushed_path = true;
                            next_action = Action::Filter;
                        } else {
                            next_pushed_path = false;
                            next_action = Action::KeepAll;
                        }
                        continue 'value;
                    }
                }
            }
        }
    }

    /// Iterative RFC 8259 validator that skips an entire JSON value in O(1) memory.
    fn skip_value<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
    ) -> std::io::Result<Result<(), PruneErr>> {
        // Each stack entry is `(kind, expect_first_or_end)`: b'{' or b'['.
        let mut stack: Vec<(u8, bool)> = Vec::new();
        'value: loop {
            let Some(b) = self.skip_ws(stream)? else {
                return Ok(Err(if stream.hit_limit {
                    PruneErr::Limit
                } else {
                    PruneErr::Eof
                }));
            };
            match b {
                b'{' => {
                    stream.next_byte()?;
                    if stack.len() >= MAX_SKIP_DEPTH {
                        return Ok(Err(PruneErr::Depth));
                    }
                    stack.push((b'{', true));
                }
                b'[' => {
                    stream.next_byte()?;
                    if stack.len() >= MAX_SKIP_DEPTH {
                        return Ok(Err(PruneErr::Depth));
                    }
                    stack.push((b'[', true));
                }
                b'"' => {
                    if let Err(e) = self.parse_string(stream, None)? {
                        return Ok(Err(e));
                    }
                }
                b'-' | b'0'..=b'9' => {
                    if let Err(e) = self.parse_number(stream, false)? {
                        return Ok(Err(e));
                    }
                }
                b't' => {
                    if let Err(e) = self.parse_literal(stream, b"true", false)? {
                        return Ok(Err(e));
                    }
                }
                b'f' => {
                    if let Err(e) = self.parse_literal(stream, b"false", false)? {
                        return Ok(Err(e));
                    }
                }
                b'n' => {
                    if let Err(e) = self.parse_literal(stream, b"null", false)? {
                        return Ok(Err(e));
                    }
                }
                _ => return Ok(Err(PruneErr::Syntax)),
            }

            loop {
                let Some((kind, first)) = stack.last_mut() else {
                    return Ok(Ok(()));
                };
                let closing = if *kind == b'{' { b'}' } else { b']' };
                let Some(next) = self.skip_ws(stream)? else {
                    return Ok(Err(if stream.hit_limit {
                        PruneErr::Limit
                    } else {
                        PruneErr::Eof
                    }));
                };
                if next == closing {
                    stream.next_byte()?;
                    stack.pop();
                    continue;
                }
                if !*first {
                    if next != b',' {
                        return Ok(Err(PruneErr::Syntax));
                    }
                    stream.next_byte()?;
                    let Some(after) = self.skip_ws(stream)? else {
                        return Ok(Err(if stream.hit_limit {
                            PruneErr::Limit
                        } else {
                            PruneErr::Eof
                        }));
                    };
                    if after == closing {
                        return Ok(Err(PruneErr::Syntax));
                    }
                }
                *first = false;
                if *kind == b'{' {
                    let Some(quote) = self.skip_ws(stream)? else {
                        return Ok(Err(if stream.hit_limit {
                            PruneErr::Limit
                        } else {
                            PruneErr::Eof
                        }));
                    };
                    if quote != b'"' {
                        return Ok(Err(PruneErr::Syntax));
                    }
                    if let Err(e) = self.parse_string(stream, None)? {
                        return Ok(Err(e));
                    }
                    let Some(colon) = self.skip_ws(stream)? else {
                        return Ok(Err(if stream.hit_limit {
                            PruneErr::Limit
                        } else {
                            PruneErr::Eof
                        }));
                    };
                    if colon != b':' {
                        return Ok(Err(PruneErr::Syntax));
                    }
                    stream.next_byte()?;
                }
                continue 'value;
            }
        }
    }

    /// Reads and validates a JSON object key. Returns `Some(raw_quoted_bytes)` if within
    /// `MAX_KEY_BYTES`, or `None` if longer (caller skips the value).
    fn read_key<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
    ) -> std::io::Result<Result<Option<Vec<u8>>, PruneErr>> {
        let mut raw = Vec::with_capacity(32);
        let mut too_long = false;
        let quote = match self.next_required(stream)? {
            Ok(b) => b,
            Err(e) => return Ok(Err(e)),
        };
        if quote != b'"' {
            return Ok(Err(PruneErr::Syntax));
        }
        raw.push(b'"');
        let mut unit = [0u8; 12];
        loop {
            let b = match self.next_required(stream)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            if b == b'"' {
                if !too_long {
                    raw.push(b'"');
                }
                return Ok(Ok((!too_long).then_some(raw)));
            }
            let len = match self.read_string_unit(stream, b, &mut unit)? {
                Ok(n) => n,
                Err(e) => return Ok(Err(e)),
            };
            if !too_long {
                if raw.len() + len <= MAX_KEY_BYTES + 1 {
                    raw.extend_from_slice(&unit[..len]);
                } else {
                    too_long = true;
                    raw.clear();
                }
            }
        }
    }

    /// Validates a JSON string, optionally emitting up to `keep_limit` content bytes into `self.out`
    /// while preserving UTF-8, escape, and UTF-16 surrogate pair boundaries.
    fn parse_string<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
        keep_limit: Option<usize>,
    ) -> std::io::Result<Result<(), PruneErr>> {
        let quote = match self.next_required(stream)? {
            Ok(b) => b,
            Err(e) => return Ok(Err(e)),
        };
        if quote != b'"' {
            return Ok(Err(PruneErr::Syntax));
        }
        if keep_limit.is_some()
            && let Err(e) = self.push_byte(b'"')
        {
            return Ok(Err(e));
        }
        let mut kept = 0usize;
        let mut unit = [0u8; 12];
        loop {
            let b = match self.next_required(stream)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            if b == b'"' {
                if keep_limit.is_some()
                    && let Err(e) = self.push_byte(b'"')
                {
                    return Ok(Err(e));
                }
                return Ok(Ok(()));
            }
            let len = match self.read_string_unit(stream, b, &mut unit)? {
                Ok(n) => n,
                Err(e) => return Ok(Err(e)),
            };
            if let Some(limit) = keep_limit
                && kept + len <= limit
            {
                if let Err(e) = self.push_slice(&unit[..len]) {
                    return Ok(Err(e));
                }
                kept += len;
            }
        }
    }

    /// Validates one atomic JSON string character/escape/surrogate-pair starting with `first`
    /// and writes its raw bytes into `unit[..len]`.
    fn read_string_unit<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
        first: u8,
        unit: &mut [u8; 12],
    ) -> std::io::Result<Result<usize, PruneErr>> {
        if first < 0x20 {
            return Ok(Err(PruneErr::Syntax));
        }
        if first == b'\\' {
            let esc = match self.next_required(stream)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            match esc {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                    unit[0] = b'\\';
                    unit[1] = esc;
                    return Ok(Ok(2));
                }
                b'u' => {
                    unit[0] = b'\\';
                    unit[1] = b'u';
                    let mut code = 0u16;
                    for slot in &mut unit[2..6] {
                        let h = match self.next_required(stream)? {
                            Ok(b) => b,
                            Err(e) => return Ok(Err(e)),
                        };
                        let Some(d) = hex_val(h) else {
                            return Ok(Err(PruneErr::Syntax));
                        };
                        code = (code << 4) | d;
                        *slot = h;
                    }
                    if (0xDC00..=0xDFFF).contains(&code) {
                        return Ok(Err(PruneErr::Syntax));
                    }
                    if (0xD800..=0xDBFF).contains(&code) {
                        let b1 = match self.next_required(stream)? {
                            Ok(b) => b,
                            Err(e) => return Ok(Err(e)),
                        };
                        let b2 = match self.next_required(stream)? {
                            Ok(b) => b,
                            Err(e) => return Ok(Err(e)),
                        };
                        if b1 != b'\\' || b2 != b'u' {
                            return Ok(Err(PruneErr::Syntax));
                        }
                        unit[6] = b'\\';
                        unit[7] = b'u';
                        let mut low = 0u16;
                        for slot in &mut unit[8..12] {
                            let h = match self.next_required(stream)? {
                                Ok(b) => b,
                                Err(e) => return Ok(Err(e)),
                            };
                            let Some(d) = hex_val(h) else {
                                return Ok(Err(PruneErr::Syntax));
                            };
                            low = (low << 4) | d;
                            *slot = h;
                        }
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return Ok(Err(PruneErr::Syntax));
                        }
                        return Ok(Ok(12));
                    }
                    return Ok(Ok(6));
                }
                _ => return Ok(Err(PruneErr::Syntax)),
            }
        }
        if first < 0x80 {
            unit[0] = first;
            return Ok(Ok(1));
        }
        // RFC 3629 UTF-8 multibyte validation.
        let (extra, second_range): (usize, (u8, u8)) = match first {
            0xC2..=0xDF => (1, (0x80, 0xBF)),
            0xE0 => (2, (0xA0, 0xBF)),
            0xE1..=0xEC | 0xEE..=0xEF => (2, (0x80, 0xBF)),
            0xED => (2, (0x80, 0x9F)),
            0xF0 => (3, (0x90, 0xBF)),
            0xF1..=0xF3 => (3, (0x80, 0xBF)),
            0xF4 => (3, (0x80, 0x8F)),
            _ => return Ok(Err(PruneErr::Syntax)),
        };
        unit[0] = first;
        for (offset, slot) in unit[1..=extra].iter_mut().enumerate() {
            let b = match self.next_required(stream)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            let (lo, hi) = if offset == 0 {
                second_range
            } else {
                (0x80, 0xBF)
            };
            if !(lo..=hi).contains(&b) {
                return Ok(Err(PruneErr::Syntax));
            }
            *slot = b;
        }
        Ok(Ok(1 + extra))
    }

    fn parse_number<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
        emit: bool,
    ) -> std::io::Result<Result<(), PruneErr>> {
        let mut count = 0usize;
        let emit_byte = |this: &mut Self, b: u8, count: &mut usize| -> Result<(), PruneErr> {
            if emit {
                if *count >= 128 {
                    return Err(PruneErr::MetadataTooLarge);
                }
                this.push_byte(b)?;
                *count += 1;
            }
            Ok(())
        };

        if stream.peek_byte()? == Some(b'-') {
            stream.next_byte()?;
            if let Err(e) = emit_byte(self, b'-', &mut count) {
                return Ok(Err(e));
            }
        }
        let first = match self.next_required(stream)? {
            Ok(b) => b,
            Err(e) => return Ok(Err(e)),
        };
        match first {
            b'0' => {
                if let Err(e) = emit_byte(self, b'0', &mut count) {
                    return Ok(Err(e));
                }
            }
            b'1'..=b'9' => {
                if let Err(e) = emit_byte(self, first, &mut count) {
                    return Ok(Err(e));
                }
                while let Some(b @ b'0'..=b'9') = stream.peek_byte()? {
                    stream.next_byte()?;
                    if let Err(e) = emit_byte(self, b, &mut count) {
                        return Ok(Err(e));
                    }
                }
            }
            _ => return Ok(Err(PruneErr::Syntax)),
        }
        if stream.peek_byte()? == Some(b'.') {
            stream.next_byte()?;
            if let Err(e) = emit_byte(self, b'.', &mut count) {
                return Ok(Err(e));
            }
            let d = match self.next_required(stream)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            if !d.is_ascii_digit() {
                return Ok(Err(PruneErr::Syntax));
            }
            if let Err(e) = emit_byte(self, d, &mut count) {
                return Ok(Err(e));
            }
            while let Some(b @ b'0'..=b'9') = stream.peek_byte()? {
                stream.next_byte()?;
                if let Err(e) = emit_byte(self, b, &mut count) {
                    return Ok(Err(e));
                }
            }
        }
        if matches!(stream.peek_byte()?, Some(b'e' | b'E')) {
            let e = stream.next_byte()?.unwrap();
            if let Err(err) = emit_byte(self, e, &mut count) {
                return Ok(Err(err));
            }
            if matches!(stream.peek_byte()?, Some(b'+' | b'-')) {
                let s = stream.next_byte()?.unwrap();
                if let Err(err) = emit_byte(self, s, &mut count) {
                    return Ok(Err(err));
                }
            }
            let d = match self.next_required(stream)? {
                Ok(b) => b,
                Err(err) => return Ok(Err(err)),
            };
            if !d.is_ascii_digit() {
                return Ok(Err(PruneErr::Syntax));
            }
            if let Err(err) = emit_byte(self, d, &mut count) {
                return Ok(Err(err));
            }
            while let Some(b @ b'0'..=b'9') = stream.peek_byte()? {
                stream.next_byte()?;
                if let Err(err) = emit_byte(self, b, &mut count) {
                    return Ok(Err(err));
                }
            }
        }
        if let Some(after) = stream.peek_byte()?
            && !matches!(after, b' ' | b'\t' | b'\r' | b'\n' | b',' | b'}' | b']')
        {
            return Ok(Err(PruneErr::Syntax));
        }
        Ok(Ok(()))
    }

    fn parse_literal<R: BufRead>(
        &mut self,
        stream: &mut LineStream<'_, R>,
        expected: &[u8],
        emit: bool,
    ) -> std::io::Result<Result<(), PruneErr>> {
        for &want in expected {
            let got = match self.next_required(stream)? {
                Ok(b) => b,
                Err(e) => return Ok(Err(e)),
            };
            if got != want {
                return Ok(Err(PruneErr::Syntax));
            }
        }
        if emit && let Err(e) = self.push_slice(expected) {
            return Ok(Err(e));
        }
        Ok(Ok(()))
    }
}

fn hex_val(b: u8) -> Option<u16> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u16),
        b'a'..=b'f' => Some((b - b'a' + 10) as u16),
        b'A'..=b'F' => Some((b - b'A' + 10) as u16),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn read_small(
        data: &[u8],
        mode: Mode,
        provider: &str,
        fast_limit: usize,
        hasher: Option<&mut Sha256>,
    ) -> LineRecord {
        let mut reader = Cursor::new(data);
        read_line_with_limits(
            &mut reader,
            mode,
            provider,
            None,
            hasher,
            fast_limit,
            1024,
            64 * 1024 * 1024,
        )
        .unwrap()
    }

    #[test]
    fn codex_oversized_response_item_prunes_payload_and_preserves_sha256() {
        let big = "A".repeat(40_000);
        let row = json!({
            "timestamp": "2026-11-07T14:40:21.123Z",
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call_1",
                "output": big,
                "encrypted_content": "E".repeat(40_000),
                "content": [{"type": "input_image", "image_url": "data:image/png;base64,".to_owned() + &"B".repeat(20_000)}]
            }
        });
        let raw = format!("{row}\n");
        let expected_hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
        let mut hasher = Sha256::new();
        let rec = read_small(
            raw.as_bytes(),
            Mode::History,
            "codex",
            128,
            Some(&mut hasher),
        );
        assert_eq!(format!("{:x}", hasher.finalize()), expected_hash);
        match rec {
            LineRecord::Complete { bytes, parsed } => {
                assert_eq!(bytes, raw.len() as u64);
                let v = parsed.unwrap();
                assert_eq!(v["type"], "response_item");
                assert_eq!(v["timestamp"], "2026-11-07T14:40:21.123Z");
                assert_eq!(v["payload"]["type"], "function_call_output");
                assert!(v["payload"].get("output").is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
        // Session mode keeps call_id and truncates output while still valid JSON.
        match read_small(raw.as_bytes(), Mode::Session, "codex", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                let v = parsed.unwrap();
                assert_eq!(v["payload"]["call_id"], "call_1");
                assert!(v["payload"]["output"].as_str().unwrap().len() <= MAX_KEPT_STRING_BYTES);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn claude_oversized_assistant_preserves_trailing_usage_and_session_tools() {
        let raw = format!(
            "{{\"type\":\"assistant\",\"uuid\":\"u1\",\"sessionId\":\"s1\",\"requestId\":\"r1\",\"timestamp\":\"2026-11-07T14:40:21Z\",\"message\":{{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-6\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"Write\",\"input\":{{\"content\":\"{}\"}}}}],\"usage\":{{\"input_tokens\":1200,\"output_tokens\":4500,\"cache_read_input_tokens\":300}}}}}}\n",
            "Z".repeat(50_000)
        );
        match read_small(raw.as_bytes(), Mode::History, "claude", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                let v = parsed.unwrap();
                assert_eq!(v["message"]["model"], "claude-sonnet-4-6");
                assert_eq!(v["message"]["usage"]["input_tokens"], 1200);
                assert_eq!(v["message"]["usage"]["output_tokens"], 4500);
                assert!(v["message"].get("content").is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
        match read_small(raw.as_bytes(), Mode::Session, "claude", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                let v = parsed.unwrap();
                assert_eq!(v["message"]["content"][0]["type"], "tool_use");
                assert_eq!(v["message"]["content"][0]["name"], "Write");
                assert_eq!(v["message"]["content"][0]["id"], "tu_1");
                assert!(v["message"]["content"][0].get("input").is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn truncation_never_splits_utf8_or_surrogate_pairs_and_rejects_bad_json() {
        // Fill near MAX_KEPT_STRING_BYTES with multibyte UTF-8 and surrogate pairs.
        let mut s = "中".repeat(5460); // 16380 bytes
        s.push('🎉'); // 4 bytes -> crosses 16384 boundary
        let raw = format!(
            "{{\"type\":\"user\",\"timestamp\":\"2026-11-07T00:00:00Z\",\"message\":{{\"content\":\"{}\"}}}}\n",
            s
        );
        match read_small(raw.as_bytes(), Mode::Session, "claude", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                let v = parsed.unwrap();
                assert!(v["message"]["content"].as_str().unwrap().starts_with("中"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Surrogate pair crossing truncation boundary.
        let esc = format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"function_call_output\",\"output\":\"{}{}\"}}}}\n",
            "a".repeat(MAX_KEPT_STRING_BYTES - 4),
            "\\uD83D\\uDE00"
        );
        match read_small(esc.as_bytes(), Mode::Session, "codex", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                let v = parsed.unwrap();
                assert_eq!(
                    v["payload"]["output"].as_str().unwrap().len(),
                    MAX_KEPT_STRING_BYTES - 4
                );
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Lone surrogate inside skipped field must still fail JSON validation.
        let bad_surrogate = format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"x\",\"output\":\"{}\\uD800\"}}}}\n",
            "a".repeat(4096)
        );
        match read_small(bad_surrogate.as_bytes(), Mode::History, "codex", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                assert_eq!(parsed, Err(LineError::InvalidJson));
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Invalid UTF-8 inside skipped field must fail JSON validation.
        let mut bad_utf8 =
            b"{\"type\":\"response_item\",\"payload\":{\"type\":\"x\",\"output\":\"".to_vec();
        bad_utf8.extend_from_slice(&vec![b'a'; 2048]);
        bad_utf8.push(0xFF);
        bad_utf8.extend_from_slice(b"\"}}\n");
        match read_small(&bad_utf8, Mode::History, "codex", 128, None) {
            LineRecord::Complete { parsed, .. } => {
                assert_eq!(parsed, Err(LineError::InvalidJson));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unterminated_garbage_reports_oversized_while_valid_json_prefix_waits() {
        let garbage = vec![b'x'; 2048];
        assert!(matches!(
            read_small(&garbage, Mode::History, "codex", 128, None),
            LineRecord::Oversized
        ));
        let in_flight = format!(
            "{{\"type\":\"response_item\",\"payload\":{{\"output\":\"{}",
            "a".repeat(2048)
        );
        assert!(matches!(
            read_small(in_flight.as_bytes(), Mode::History, "codex", 128, None),
            LineRecord::Incomplete { .. }
        ));
    }
}
