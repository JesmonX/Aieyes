//! Loopback-only Responses forwarding. The journal contains usage, never prompts or keys.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub fn journal(root: &Path) -> Result<rusqlite::Connection> {
    crate::codex_auth::files::safe(&root.join("meter.sqlite"))?;
    let db = rusqlite::Connection::open(root.join("meter.sqlite"))?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY,stamp INTEGER NOT NULL,payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS requests_stamp ON requests(stamp);")?;
    db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS meter_sequence(id INTEGER PRIMARY KEY,value INTEGER NOT NULL); INSERT OR IGNORE INTO meter_sequence VALUES(1,0);")?;
    let columns = db
        .prepare("PRAGMA table_info(requests)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|v| v == "revision") {
        db.execute_batch("ALTER TABLE requests ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;")?;
    }
    db.execute_batch("COMMIT;")?;
    Ok(db)
}
pub(super) fn save(root: &Path, value: &Value) -> Result<()> {
    let mut db = journal(root)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("UPDATE meter_sequence SET value=value+1 WHERE id=1", [])?;
    tx.execute("INSERT INTO requests(id,stamp,payload,revision) VALUES(?1,?2,?3,(SELECT value FROM meter_sequence WHERE id=1)) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,revision=excluded.revision", rusqlite::params![value["id"].as_str(),value["timestamp"].as_i64(),value.to_string()])?;
    tx.commit()?;
    Ok(())
}
pub fn records(root: &Path, after: i64) -> Result<Value> {
    if !root.join("meter.sqlite").exists() {
        return Ok(json!({"records":[],"cursor":0}));
    }
    let db = journal(root)?;
    let tx = db.unchecked_transaction()?;
    let cursor: i64 = tx.query_row("SELECT value FROM meter_sequence WHERE id=1", [], |r| {
        r.get(0)
    })?;
    let mut query =
        tx.prepare("SELECT payload FROM requests WHERE revision>?1 ORDER BY revision,id")?;
    let rows = query
        .query_map([after], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|v| serde_json::from_str::<Value>(&v))
        .collect::<serde_json::Result<Vec<_>>>()?;
    Ok(json!({"records":rows,"cursor":cursor}))
}
pub(super) fn interrupt_pending(root: &Path, reason: &str) -> Result<()> {
    for mut record in records(root, -1)?["records"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if record["status"] == "pending" {
            record["status"] = json!("incomplete");
            record["errorCode"] = json!(reason);
            record["endedAt"] = json!(now());
            save(root, &record)?;
        }
    }
    Ok(())
}
fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}
fn reply(stream: &mut TcpStream, code: u16, value: Value) -> Result<()> {
    let body = value.to_string();
    write!(
        stream,
        "HTTP/1.1 {code} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}
#[derive(Default)]
pub(crate) struct UsageReader {
    pending: Vec<u8>,
    data: Vec<u8>,
    pub usage: Option<Value>,
    pub response_id: Option<String>,
    pub model: Option<String>,
    pub complete: bool,
}
impl UsageReader {
    pub(crate) fn value(&mut self, value: &Value) {
        let response = value.get("response").unwrap_or(value);
        if let Some(id) = response["id"].as_str() {
            self.response_id = Some(id.into());
        }
        if let Some(model) = response["model"].as_str() {
            self.model = Some(model.into());
        }
        if matches!(
            value["type"].as_str(),
            Some("response.completed" | "response.incomplete" | "response.failed")
        ) || response["object"] == "response"
            && matches!(
                response["status"].as_str(),
                Some("completed" | "incomplete" | "failed")
            )
            || response["object"] == "response.compaction"
        {
            self.complete = value["type"] == "response.completed"
                || response["status"] == "completed"
                || response["object"] == "response.compaction";
            if let Some(usage) = response.get("usage").filter(|v| v.is_object()) {
                self.usage = Some(usage.clone());
            }
        }
    }
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            if line == b"\n" || line == b"\r\n" {
                self.dispatch();
            } else if let Some(data) = line
                .strip_prefix(b"data: ")
                .or_else(|| line.strip_prefix(b"data:"))
            {
                self.data.extend_from_slice(data);
                if self.data.len() > 8 * 1024 * 1024 {
                    self.data.clear();
                }
            }
        }
        if self.pending.len() > 8 * 1024 * 1024 {
            self.pending.clear();
        }
    }
    fn dispatch(&mut self) {
        if let Ok(value) = serde_json::from_slice::<Value>(&self.data) {
            self.value(&value);
        }
        self.data.clear();
    }
    fn finish(&mut self) {
        self.feed(b"\n\n");
    }
}
pub(super) fn handle(root: &Path, mut stream: TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let request: Vec<_> = line.split_whitespace().collect();
    if request.len() != 3 {
        return reply(&mut stream, 400, json!({"error":"Invalid request"}));
    }
    let method = request[0].to_owned();
    let path = request[1].to_owned();
    let mut headers = std::collections::BTreeMap::<String, String>::new();
    let mut size = line.len();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        size += n;
        ensure!(n > 0 && size < 65536, "请求头无效");
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (key, value) = line.split_once(':').context("请求头无效")?;
        headers.insert(key.trim().to_ascii_lowercase(), value.trim().into());
    }
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() < 5 || segments[1] != "providers" || segments[3] != "v1" {
        return reply(&mut stream, 404, json!({"error":"Unknown route"}));
    }
    let id = identifier(segments[2])?;
    let config: PrivateConnection =
        match read_json(&root.join("connections").join(format!("{id}.json"))) {
            Ok(v) => v,
            Err(_) => return reply(&mut stream, 404, json!({"error":"Unknown connection"})),
        };
    if !equal(
        headers
            .get("authorization")
            .map(String::as_str)
            .unwrap_or(""),
        &format!("Bearer {}", config.local_token),
    ) {
        return reply(
            &mut stream,
            401,
            json!({"error":"Local authentication required"}),
        );
    }
    let route = segments[4..].join("/");
    if route == "health" && method == "GET" {
        return reply(&mut stream, 200, json!({"ready":true}));
    }
    if !matches!(
        (method.as_str(), route.as_str()),
        ("POST", "responses" | "responses/compact") | ("GET", "models")
    ) {
        return reply(
            &mut stream,
            404,
            json!({"error":"Unsupported Responses route"}),
        );
    }
    if headers.contains_key("transfer-encoding") {
        return reply(&mut stream, 411, json!({"error":"Content-Length required"}));
    }
    let length = headers
        .get("content-length")
        .map(|v| v.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    if length > 32 * 1024 * 1024 {
        return reply(&mut stream, 413, json!({"error":"Request too large"}));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    // Forward encoded requests unchanged; the response can provide the actual model.
    let input = if headers
        .get("content-encoding")
        .is_none_or(|v| v == "identity")
    {
        serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let metadata: Value = headers
        .get("x-codex-turn-metadata")
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or(Value::Null);
    let mut record = json!({"id":new_id(),"connectionId":id,"sourceId":config.connection.source_id,"timestamp":now(),"status":"pending","sessionId":metadata["session_id"].as_str().or_else(||headers.get("session-id").map(String::as_str)),"turnId":metadata["turn_id"].as_str(),"model":input["model"].as_str().unwrap_or(""),"prices":config.connection.prices});
    let metered = method == "POST";
    if metered {
        save(root, &record)?;
    }
    let mut headers_sent = false;
    let mut usage = UsageReader::default();
    let result = (|| -> Result<()> {
        let client = client(&config.connection.proxy, &config.proxy_environment)?;
        let url = format!(
            "{}/{}",
            config.connection.base_url.trim_end_matches('/'),
            route
        );
        let mut request = client
            .request(reqwest::Method::from_bytes(method.as_bytes())?, url)
            .bearer_auth(&config.api_key)
            .header("accept-encoding", "identity");
        // Forward protocol and correlation headers, never client credentials, cookies or Host.
        for (key, value) in &headers {
            if key == "content-type"
                || key == "content-encoding"
                || key == "accept"
                || key == "originator"
                || key == "session-id"
                || key == "thread-id"
                || key.starts_with("x-codex-")
                || key.starts_with("x-openai-")
                || key == "x-client-request-id"
            {
                request = request.header(key, value);
            }
        }
        let mut upstream = request.body(body).send()?;
        let status = upstream.status();
        record["httpStatus"] = json!(status.as_u16());
        let content_type = upstream
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_owned();
        write!(
            stream,
            "HTTP/1.1 {} Response\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n",
            status.as_u16(),
            content_type
        )?;
        for (key, value) in upstream.headers() {
            if (matches!(
                key.as_str(),
                "retry-after" | "x-request-id" | "openai-processing-ms" | "content-encoding"
            ) || key.as_str().starts_with("x-ratelimit-"))
                && let Ok(value) = value.to_str()
            {
                write!(stream, "{key}: {value}\r\n")?;
            }
        }
        stream.write_all(b"\r\n")?;
        headers_sent = true;
        let mut json_body = Vec::new();
        let mut chunk = [0; 16384];
        let mut connected = true;
        loop {
            let size = upstream.read(&mut chunk)?;
            if size == 0 {
                break;
            }
            if content_type.contains("event-stream") {
                usage.feed(&chunk[..size]);
            } else if json_body.len() + size <= 8 * 1024 * 1024 {
                json_body.extend_from_slice(&chunk[..size]);
            }
            if connected {
                connected = (|| -> std::io::Result<()> {
                    write!(stream, "{size:x}\r\n")?;
                    stream.write_all(&chunk[..size])?;
                    stream.write_all(b"\r\n")?;
                    stream.flush()
                })()
                .is_ok();
            }
        }
        let _ = stream.write_all(b"0\r\n\r\n");
        usage.finish();
        if let Ok(value) = serde_json::from_slice::<Value>(&json_body) {
            usage.value(&value);
        }
        record["responseId"] = json!(usage.response_id);
        record["usage"] = json!(usage.usage);
        if let Some(model) = &usage.model {
            record["model"] = json!(model);
        }
        record["status"] = json!(if !status.is_success() {
            "rejected"
        } else if usage.complete && super::usage::response_tokens(&record["usage"]).is_some() {
            "completed"
        } else {
            "incomplete"
        });
        Ok(())
    })();
    // Keep terminal usage even when the transport fails after it was delivered.
    record["responseId"] = json!(usage.response_id);
    record["usage"] = json!(usage.usage);
    if let Some(model) = &usage.model {
        record["model"] = json!(model);
    }
    if result.is_err() {
        record["status"] = json!("incomplete");
        record["errorCode"] = json!("upstream-or-stream");
        if !headers_sent {
            let _ = reply(
                &mut stream,
                502,
                json!({"error":"API connection or stream interrupted"}),
            );
        }
    }
    if metered {
        record["endedAt"] = json!(now());
        save(root, &record)?;
    }
    Ok(())
}
pub fn serve(root: &Path) -> Result<()> {
    private_dir(root)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("service.lock"))?;
    let Ok(_lock) = crate::file_lock::FileLock::try_exclusive(lock) else {
        return Ok(());
    };
    let runtime: Runtime = read_json(&root.join("runtime.json"))?;
    let listener =
        TcpListener::bind(("127.0.0.1", runtime.port)).context("API 计量服务端口不可用")?;
    journal(root)?;
    interrupt_pending(root, "service-restarted")?;
    listener.set_nonblocking(true)?;
    let mut requests = Vec::new();
    while has_connections(root) {
        match listener.accept() {
            Ok((stream, _)) => {
                let root = root.to_path_buf();
                requests.push(std::thread::spawn(move || {
                    let _ = handle(&root, stream);
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e.into()),
        }
        requests.retain(|r| !r.is_finished());
    }
    drop(listener);
    for request in requests {
        let _ = request.join();
    }
    Ok(())
}
