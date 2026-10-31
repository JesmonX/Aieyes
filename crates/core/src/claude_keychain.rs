//! Claude Code official quota on macOS needs the OAuth token that Claude keeps in the
//! login Keychain (`Claude Code-credentials`). Aieyes asks for it once after the first
//! launch, keeps the token in process memory only and persists just the consent.
//!
//! The read goes through `/usr/bin/security` on purpose: builds without a signing
//! identity are ad-hoc signed (`signingIdentity "-"`), so an "Always Allow" granted to
//! the app binary would be lost on every update, while the grant to `security`
//! survives. Claude rewrites the entry when it refreshes its token, which can make
//! macOS ask again (best effort).
//!
//! The macOS app runs one core process per lane, so the request runs on the quotas
//! lane, next to the queries that use the token; quota queries wait while the dialog
//! is open. Any other process that queries the quota (estimates, automatic sampling)
//! reads the item itself, but only after a recorded grant. A read is reserved under
//! the lock and runs outside it, so concurrent queries never open a second dialog.
use crate::models::now;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const SERVICE: &str = "Claude Code-credentials";
/// A token that Claude has since replaced is re-read at most this often.
pub const RETRY_SECONDS: i64 = 24 * 60 * 60;
/// How long the user has to answer the system dialog.
const DIALOG_SECONDS: u64 = 120;
const EXPIRED: &str = "Claude Code 登录已过期，打开 Claude Code 一次即可刷新";

/// Persisted in settings as `claudeKeychain`. `result` is granted, denied or missing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Consent {
    pub asked_at: i64,
    pub granted: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub result: String,
}

#[derive(Default)]
pub struct Cache {
    token: Option<String>,
    /// Newest mtime of Claude's credential files at the last read.
    stamp: i64,
    /// When this process last read the item; 0 before its first read.
    read_at: i64,
    /// Last read made after a rejected token or a failed read.
    retried_at: i64,
    /// Outcome of this process's last read.
    result: &'static str,
    /// The token was rejected; waiting for Claude to refresh its own login.
    expired: bool,
    /// A read is in progress (its dialog may be open); nobody starts another.
    reading: bool,
}

pub enum Read {
    Token(String),
    Missing,
    Denied,
    /// No answer before the timeout, no UI available, or the read was cancelled.
    Unanswered,
}

/// Reads the Keychain item. Injected in tests so they never touch the real Keychain.
pub type Runner<'a> = &'a dyn Fn() -> Read;

/// What a query does next, decided under the lock.
enum Next {
    Token(String),
    Read,
    Fail(&'static str),
}

#[derive(Debug)]
struct Unavailable(&'static str);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Unavailable {}

/// A missing or declined optional grant is a notice, not a failed server read.
pub(crate) fn quota_notice(error: &anyhow::Error) -> Option<String> {
    error.downcast_ref::<Unavailable>().map(|v| v.0.to_owned())
}

fn lock() -> MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    // A reserved read must always be finished, so a poisoned lock is recovered.
    CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn cached() -> Option<String> {
    lock().token.clone()
}

fn claude_root() -> PathBuf {
    crate::store::home().join(".claude")
}

/// `AIEYES_AUTO_SCAN=off` (UI fixtures, see `Engine::spawn_auto_scan`) also skips the
/// launch-time read, so tests never read the developer's real Keychain.
fn launch_reads_disabled() -> bool {
    std::env::var("AIEYES_AUTO_SCAN").is_ok_and(|v| v == "off")
}

/// Only the default directory's login is stored under the unsuffixed item name.
pub fn default_dir(path: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canonical(path) == canonical(&claude_root())
}

/// Claude rewrites one of these when it refreshes its login.
fn stamp(home: &Path) -> i64 {
    [
        home.join(".claude/.credentials.json"),
        home.join(".claude.json"),
    ]
    .iter()
    .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
    .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    .map(|d| d.as_secs() as i64)
    .max()
    .unwrap_or(0)
}

fn token(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?.trim();
    // `security -w` prints data it considers unprintable as hex.
    let hex = || -> Option<Vec<u8>> {
        if text.is_empty() || text.len() % 2 != 0 {
            return None;
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
            .collect()
    };
    let v: Value = serde_json::from_str(text)
        .ok()
        .or_else(|| serde_json::from_slice(&hex()?).ok())?;
    v["claudeAiOauth"]["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(Into::into)
}

/// `security` exits with the low byte of the OSStatus: 44 item not found,
/// 36 user interaction not allowed. Timeouts and cancellations carry no code.
/// `error` is a `process::run` failure, which ends in "（{code}）" for an exit code.
fn classify(error: &str) -> Read {
    let code = error
        .rsplit_once('（')
        .and_then(|(_, s)| s.strip_suffix('）'))
        .and_then(|s| s.parse::<i32>().ok());
    match code {
        Some(44) => Read::Missing,
        Some(36) | None => Read::Unanswered,
        Some(_) => Read::Denied,
    }
}

/// The real Keychain read. Looking the item up without `-w` reads only its attributes,
/// which never prompts, so "not signed in" is told apart from a refused dialog.
pub fn security() -> Read {
    #[cfg(target_os = "macos")]
    {
        let exists = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SERVICE])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match exists {
            Ok(status) if status.success() => {}
            Ok(status) if status.code() == Some(44) => return Read::Missing,
            _ => return Read::Unanswered,
        }
        let mut cmd = std::process::Command::new("/usr/bin/security");
        cmd.args(["find-generic-password", "-s", SERVICE, "-w"]);
        return match crate::process::run(
            cmd,
            vec![],
            std::time::Duration::from_secs(DIALOG_SECONDS),
        ) {
            Ok(out) => token(&out).map(Read::Token).unwrap_or(Read::Missing),
            Err(error) => classify(&error.to_string()),
        };
    }
    #[allow(unreachable_code)]
    Read::Missing
}

/// Records one Keychain read in memory and returns the consent to persist.
/// An unanswered read is not a refusal; callers keep the earlier consent.
fn record(cache: &mut Cache, read: Read, stamp: i64, at: i64) -> Consent {
    let result = match read {
        Read::Token(t) => {
            cache.token = Some(t);
            "granted"
        }
        Read::Missing => "missing",
        Read::Denied => "denied",
        Read::Unanswered => "unanswered",
    };
    if result != "granted" {
        cache.token = None;
        // A failed read is retried only after Claude rewrites its files, at most daily.
        cache.retried_at = at;
    }
    cache.stamp = stamp;
    cache.read_at = at;
    cache.result = result;
    cache.expired = false;
    cache.reading = false;
    Consent {
        asked_at: at,
        granted: result == "granted",
        result: result.into(),
    }
}

/// One explicit Keychain read (startup or the user's retry).
pub fn ask(cache: &mut Cache, run: Runner, stamp: i64, at: i64) -> Consent {
    record(cache, run(), stamp, at)
}

/// Startup reads once per process: when never asked, when Claude had no Keychain item
/// yet (the lookup itself never prompts), or silently again after an earlier grant
/// (the token lives in memory only). A refusal is never asked again automatically.
pub fn should_ask_at_startup(
    consent: Option<&Consent>,
    cache: &Cache,
    claude_exists: bool,
) -> bool {
    cfg!(target_os = "macos")
        && claude_exists
        && cache.token.is_none()
        && cache.read_at == 0
        && !cache.reading
        && consent.is_none_or(|c| c.asked_at == 0 || c.granted || c.result == "missing")
}

/// A query may read the item itself only after a recorded grant: once per process,
/// then again only after Claude rewrote its files, at most once per 24h.
fn query_due(consent: Option<&Consent>, cache: &Cache, stamp: i64, at: i64) -> bool {
    consent.is_some_and(|c| c.granted)
        && cache.token.is_none()
        && !cache.reading
        && (cache.read_at == 0 || stamp > cache.stamp && at - cache.retried_at >= RETRY_SECONDS)
}

/// Why a query has no token, as shown on the quota card.
fn reason(consent: Option<&Consent>, cache: &Cache) -> &'static str {
    if cache.expired {
        return EXPIRED;
    }
    let result = if cache.read_at != 0 {
        cache.result
    } else {
        consent.map_or("", |c| c.result.as_str())
    };
    match result {
        "denied" => "已拒绝钥匙串授权，仍可查看本地用量",
        "missing" => "未登录或使用 API key，仍可查看本地用量",
        _ => "未授权读取钥匙串，仍可查看本地用量",
    }
}

fn begin_query(cache: &mut Cache, consent: Option<&Consent>, stamp: i64, at: i64) -> Next {
    if let Some(token) = &cache.token {
        return Next::Token(token.clone());
    }
    if !query_due(consent, cache, stamp, at) {
        return Next::Fail(reason(consent, cache));
    }
    cache.reading = true;
    if cache.read_at != 0 {
        cache.retried_at = at;
    }
    Next::Read
}

fn finish_query(
    cache: &mut Cache,
    consent: Option<&Consent>,
    read: Read,
    stamp: i64,
    at: i64,
) -> Result<String> {
    record(cache, read, stamp, at);
    cache
        .token
        .clone()
        .ok_or_else(|| Unavailable(reason(consent, cache)).into())
}

/// The token for one quota query, reading the item when `query_due`.
pub fn for_query(
    cache: &mut Cache,
    consent: Option<&Consent>,
    run: Runner,
    stamp: i64,
    at: i64,
) -> Result<String> {
    match begin_query(cache, consent, stamp, at) {
        Next::Token(token) => Ok(token),
        Next::Fail(reason) => Err(Unavailable(reason).into()),
        Next::Read => finish_query(cache, consent, run(), stamp, at),
    }
}

/// The token for a quota query in this process.
pub fn query_token(consent: Option<&Consent>) -> Result<String> {
    let (stamp, at) = (stamp(&crate::store::home()), now());
    let next = begin_query(&mut lock(), consent, stamp, at);
    match next {
        Next::Token(token) => Ok(token),
        Next::Fail(reason) => Err(Unavailable(reason).into()),
        // Read without holding the lock: the system dialog can stay open for minutes.
        Next::Read => {
            let read = security();
            finish_query(&mut lock(), consent, read, stamp, at)
        }
    }
}

/// After a 401 Aieyes waits for Claude to refresh its own login. Only when Claude has
/// rewritten its credential files is the Keychain read again, at most once per 24h.
fn begin_retry(cache: &mut Cache, rejected: &str, stamp: i64, at: i64) -> Next {
    if let Some(token) = cache.token.clone().filter(|t| t != rejected) {
        // Another query already read the refreshed login.
        return Next::Token(token);
    }
    if cache.reading || stamp <= cache.stamp || at - cache.retried_at < RETRY_SECONDS {
        cache.token = None;
        cache.expired = true;
        return Next::Fail(EXPIRED);
    }
    cache.reading = true;
    cache.retried_at = at;
    Next::Read
}

fn finish_retry(
    cache: &mut Cache,
    read: Read,
    rejected: &str,
    stamp: i64,
    at: i64,
) -> Option<String> {
    record(cache, read, stamp, at);
    if cache.token.as_deref() == Some(rejected) {
        // Claude has not refreshed yet; the rewrite was something else.
        cache.token = None;
    }
    if cache.token.is_none() {
        cache.expired = true;
    }
    cache.token.clone()
}

/// The refreshed token to retry with, or None to wait for Claude.
pub fn after_unauthorized(
    cache: &mut Cache,
    rejected: &str,
    run: Runner,
    stamp: i64,
    at: i64,
) -> Option<String> {
    match begin_retry(cache, rejected, stamp, at) {
        Next::Token(token) => Some(token),
        Next::Fail(_) => None,
        Next::Read => finish_retry(cache, run(), rejected, stamp, at),
    }
}

/// Called by the quota query when `rejected` got a 401.
pub fn unauthorized(rejected: &str) -> Option<String> {
    let (stamp, at) = (stamp(&crate::store::home()), now());
    let next = begin_retry(&mut lock(), rejected, stamp, at);
    match next {
        Next::Token(token) => Some(token),
        Next::Fail(_) => None,
        Next::Read => {
            let read = security();
            finish_retry(&mut lock(), read, rejected, stamp, at)
        }
    }
}

/// The user turned official quota off: forget the token and record a refusal.
pub fn dismiss(cache: &mut Cache, at: i64) -> Consent {
    cache.token = None;
    cache.expired = false;
    cache.result = "denied";
    cache.read_at = cache.read_at.max(at);
    Consent {
        asked_at: at,
        granted: false,
        result: "denied".into(),
    }
}

fn status(consent: Option<&Consent>) -> Value {
    let available = cached().is_some();
    json!({
        "supported": cfg!(target_os = "macos"),
        "asked": consent.is_some_and(|c| c.asked_at > 0),
        "askedAt": consent.map(|c| c.asked_at).filter(|t| *t > 0),
        "granted": consent.is_some_and(|c| c.granted),
        "result": consent.map(|c| c.result.as_str()).filter(|r| !r.is_empty()),
        "available": available,
    })
}

impl crate::Engine {
    /// `claudeAuth.keychain` with `startup:true` follows the once-only rule; without it
    /// the user asked to retry. `dismiss:true` records a refusal without reading.
    pub(crate) fn claude_auth_call(&mut self, method: &str, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        if method == "claudeAuth.status" {
            return Ok(status(settings.claude_keychain.as_ref()));
        }
        anyhow::ensure!(method == "claudeAuth.keychain", "未知 Claude 授权操作");
        if !cfg!(target_os = "macos") {
            return Ok(status(settings.claude_keychain.as_ref()));
        }
        let consent = if params["dismiss"].as_bool() == Some(true) {
            dismiss(&mut lock(), now())
        } else {
            let skip = {
                let mut cache = lock();
                let startup = params["startup"].as_bool() == Some(true);
                // A dialog that is already open answers this request too.
                let skip = cache.reading
                    || startup
                        && (launch_reads_disabled()
                            || !should_ask_at_startup(
                                settings.claude_keychain.as_ref(),
                                &cache,
                                claude_root().is_dir(),
                            ));
                cache.reading |= !skip;
                skip
            };
            // `status` reads the cache too, so the guard above is released first.
            if skip {
                return Ok(status(settings.claude_keychain.as_ref()));
            }
            // Read without holding the lock: the system dialog can stay open for
            // minutes and quota queries must not wait on it.
            let read = security();
            record(&mut lock(), read, stamp(&crate::store::home()), now())
        };
        // No answer (timeout or no UI) is asked again at the next launch.
        if consent.result != "unanswered" {
            let tx = self.store.write_transaction()?;
            let mut latest = self.store.settings()?;
            latest.claude_keychain = Some(consent);
            self.store.save_auth_settings(&latest)?;
            tx.commit()?;
        }
        Ok(status(self.store.settings()?.claude_keychain.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn granted() -> Read {
        Read::Token("fixture-token".into())
    }

    fn consent(result: &str) -> Consent {
        Consent {
            asked_at: 1,
            granted: result == "granted",
            result: result.into(),
        }
    }

    #[test]
    fn startup_asks_only_once_and_never_after_refusal() {
        let mut cache = Cache::default();
        let mac = cfg!(target_os = "macos");
        assert_eq!(should_ask_at_startup(None, &cache, true), mac);
        assert!(!should_ask_at_startup(None, &cache, false));
        let calls = Cell::new(0);
        let deny = || {
            calls.set(calls.get() + 1);
            Read::Denied
        };
        let consent = ask(&mut cache, &deny, 1, 100);
        assert_eq!(calls.get(), 1);
        assert!(!consent.granted);
        assert_eq!(consent.result, "denied");
        assert!(!should_ask_at_startup(Some(&consent), &cache, true));
        // Next launch: a refusal stays final.
        assert!(!should_ask_at_startup(
            Some(&consent),
            &Cache::default(),
            true
        ));
        let missing = ask(&mut cache, &|| Read::Missing, 1, 100);
        // Once per process; a later launch looks again because the lookup never prompts.
        assert!(!should_ask_at_startup(Some(&missing), &cache, true));
        assert_eq!(
            should_ask_at_startup(Some(&missing), &Cache::default(), true),
            mac
        );
        let ok = ask(&mut cache, &granted, 5, 200);
        assert!(ok.granted && cache.token.as_deref() == Some("fixture-token"));
        // Already holding the token: nothing to ask this run.
        assert!(!should_ask_at_startup(Some(&ok), &cache, true));
        // Next launch, empty memory: a previous grant reads silently again.
        assert_eq!(
            should_ask_at_startup(Some(&ok), &Cache::default(), true),
            mac
        );
    }

    #[test]
    fn unanswered_dialog_is_not_a_refusal() {
        let mut cache = Cache::default();
        let answer = ask(&mut cache, &|| Read::Unanswered, 1, 100);
        assert_eq!(answer.result, "unanswered");
        assert!(!answer.granted && cache.token.is_none());
        let exit = |code| classify(&crate::process::classify_error("", code));
        assert!(matches!(classify("查询超时"), Read::Unanswered));
        assert!(matches!(exit(None), Read::Unanswered));
        assert!(matches!(exit(Some(36)), Read::Unanswered));
        assert!(matches!(exit(Some(44)), Read::Missing));
        assert!(matches!(exit(Some(51)), Read::Denied));
        assert!(matches!(exit(Some(128)), Read::Denied));
    }

    #[test]
    fn queries_read_only_after_a_grant_and_once_per_process() {
        let calls = Cell::new(0);
        let run = || {
            calls.set(calls.get() + 1);
            granted()
        };
        // Never asked, refused or missing: a query never opens the dialog itself.
        for c in [None, Some(consent("denied")), Some(consent("missing"))] {
            let mut cache = Cache::default();
            let error = for_query(&mut cache, c.as_ref(), &run, 1, 100).unwrap_err();
            assert!(error.to_string().ends_with("仍可查看本地用量"), "{error}");
        }
        assert_eq!(calls.get(), 0);
        let ok = consent("granted");
        // Another process (or a restarted one) reads once after the recorded grant.
        let mut cache = Cache::default();
        assert_eq!(
            for_query(&mut cache, Some(&ok), &run, 1, 100).unwrap(),
            "fixture-token"
        );
        assert_eq!(
            for_query(&mut cache, Some(&ok), &run, 1, 200).unwrap(),
            "fixture-token"
        );
        assert_eq!(calls.get(), 1);
        // A failed read waits for Claude to rewrite its files, at most daily.
        let mut cache = Cache::default();
        let deny = || {
            calls.set(calls.get() + 1);
            Read::Denied
        };
        let error = for_query(&mut cache, Some(&ok), &deny, 1, 100).unwrap_err();
        assert_eq!(error.to_string(), "已拒绝钥匙串授权，仍可查看本地用量");
        assert!(for_query(&mut cache, Some(&ok), &run, 1, 100 + RETRY_SECONDS).is_err());
        assert!(for_query(&mut cache, Some(&ok), &run, 2, 200).is_err());
        assert_eq!(calls.get(), 2);
        assert!(for_query(&mut cache, Some(&ok), &run, 2, 100 + RETRY_SECONDS).is_ok());
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn a_reserved_read_is_never_started_twice() {
        let ok = consent("granted");
        let mut cache = Cache::default();
        // The first query reserves the read; while its dialog is open, others fail fast.
        assert!(matches!(
            begin_query(&mut cache, Some(&ok), 1, 100),
            Next::Read
        ));
        assert!(matches!(
            begin_query(&mut cache, Some(&ok), 1, 100),
            Next::Fail(_)
        ));
        assert!(!should_ask_at_startup(Some(&ok), &cache, true));
        assert_eq!(
            finish_query(&mut cache, Some(&ok), granted(), 1, 100).unwrap(),
            "fixture-token"
        );
        assert!(matches!(
            begin_query(&mut cache, Some(&ok), 1, 100),
            Next::Token(_)
        ));
        // Two 401s at once: one retry read; the other waits.
        let at = 100 + RETRY_SECONDS;
        assert!(matches!(
            begin_retry(&mut cache, "fixture-token", 2, at),
            Next::Read
        ));
        assert!(matches!(
            begin_retry(&mut cache, "fixture-token", 2, at),
            Next::Fail(_)
        ));
        let refreshed = Read::Token("refreshed".into());
        assert_eq!(
            finish_retry(&mut cache, refreshed, "fixture-token", 2, at).as_deref(),
            Some("refreshed")
        );
        // A query that was rejected with the old token picks up the new one.
        assert!(matches!(
            begin_retry(&mut cache, "fixture-token", 2, at),
            Next::Token(t) if t == "refreshed"
        ));
    }

    #[test]
    fn unauthorized_rereads_only_after_claude_rewrites_and_at_most_daily() {
        let mut cache = Cache::default();
        ask(&mut cache, &granted, 10, 1_000);
        let calls = Cell::new(0);
        let run = || {
            calls.set(calls.get() + 1);
            Read::Token("refreshed".into())
        };
        // Claude has not rewritten its files: wait, do not ask.
        assert!(after_unauthorized(&mut cache, "fixture-token", &run, 10, 2_000).is_none());
        assert!(cache.token.is_none());
        assert_eq!(calls.get(), 0);
        let ok = consent("granted");
        // Queries keep waiting too, and say why.
        let error = for_query(&mut cache, Some(&ok), &run, 10, 3_000).unwrap_err();
        assert_eq!(error.to_string(), EXPIRED);
        assert_eq!(calls.get(), 0);
        // Claude rewrote its files: one read, and the query retries with the new token.
        let at = 2_000 + RETRY_SECONDS;
        assert_eq!(
            after_unauthorized(&mut cache, "fixture-token", &run, 20, at).as_deref(),
            Some("refreshed")
        );
        assert_eq!(calls.get(), 1);
        // The new token is rejected again within 24h: throttled.
        assert!(after_unauthorized(&mut cache, "refreshed", &run, 30, at + 60).is_none());
        assert!(for_query(&mut cache, Some(&ok), &run, 30, at + 120).is_err());
        assert_eq!(calls.get(), 1);
        // Still the rejected token after a day: Claude has not refreshed; keep waiting.
        assert!(
            after_unauthorized(&mut cache, "refreshed", &run, 40, at + RETRY_SECONDS).is_none()
        );
        assert_eq!(calls.get(), 2);
        assert_eq!(
            for_query(&mut cache, Some(&ok), &run, 40, at + RETRY_SECONDS + 1)
                .unwrap_err()
                .to_string(),
            EXPIRED
        );
    }

    #[test]
    fn dismiss_forgets_the_token() {
        let mut cache = Cache::default();
        ask(&mut cache, &granted, 1, 100);
        let refused = dismiss(&mut cache, 200);
        assert!(cache.token.is_none() && !refused.granted);
        assert_eq!(refused.result, "denied");
        assert!(!should_ask_at_startup(
            Some(&refused),
            &Cache::default(),
            true
        ));
        let error = for_query(&mut cache, Some(&refused), &granted, 2, 300).unwrap_err();
        assert_eq!(error.to_string(), "已拒绝钥匙串授权，仍可查看本地用量");
    }

    #[test]
    fn consent_never_serializes_a_token() {
        let mut cache = Cache::default();
        let consent = ask(&mut cache, &granted, 1, 7);
        let text = serde_json::to_string(&consent).unwrap();
        assert!(!text.contains("fixture-token"));
        assert_eq!(text, r#"{"askedAt":7,"granted":true,"result":"granted"}"#);
        assert_eq!(token(br#"{"claudeAiOauth":{"accessToken":""}}"#), None);
        let plain = br#"{"claudeAiOauth":{"accessToken":"t"}}"#;
        assert_eq!(token(plain).as_deref(), Some("t"));
        let hex: String = plain.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(token(format!("{hex}\n").as_bytes()).as_deref(), Some("t"));
        assert_eq!(token(b"not a login"), None);
    }
}
