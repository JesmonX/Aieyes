//! Update coordination is independent of engine/SQLite locks.
use anyhow::{Result, ensure};
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Registry {
    preparing: bool,
    tasks: Vec<std::sync::Weak<Task>>,
}
struct Task {
    method: String,
    query_id: Mutex<String>,
    cancel: AtomicBool,
    query_cancelled: AtomicBool,
    critical: AtomicUsize,
    protected: bool,
    cancellable: bool,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
thread_local! { static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(Default::default)
}
#[derive(Clone)]
pub struct Context(Arc<Task>);
impl Context {
    pub(crate) fn cancel(&self) {
        self.0.cancel.store(true, Ordering::Release);
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.cancel.load(Ordering::Acquire)
    }
}
pub(crate) fn is_preparing() -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .preparing
}
pub struct Scope {
    previous: Option<Context>,
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|v| *v.borrow_mut() = self.previous.take());
    }
}
pub fn context() -> Option<Context> {
    CURRENT.with(|v| v.borrow().clone())
}
pub fn inherit(context: Option<Context>) -> Scope {
    Scope {
        previous: CURRENT.with(|v| v.replace(context)),
    }
}
pub fn enter(method: &str) -> Result<Scope> {
    if context().is_some() {
        check()?;
        return Ok(inherit(context()));
    }
    let behavior = crate::contracts::method(method).map_or("finish", |m| m.update_behavior);
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    ensure!(
        !r.preparing || matches!(behavior, "observe" | "complete"),
        "正在准备更新，请稍候"
    );
    let task = Arc::new(Task {
        method: method.into(),
        query_id: Mutex::new(String::new()),
        cancel: AtomicBool::new(false),
        query_cancelled: AtomicBool::new(false),
        critical: AtomicUsize::new(0),
        protected: matches!(behavior, "finish" | "complete"),
        cancellable: behavior == "cancel",
    });
    r.tasks.retain(|t| t.strong_count() > 0);
    r.tasks.push(Arc::downgrade(&task));
    Ok(inherit(Some(Context(task))))
}
pub fn check() -> Result<()> {
    ensure!(
        !context().is_some_and(|c| c.0.query_cancelled.load(Ordering::Acquire)),
        "查询已取消"
    );
    ensure!(
        !context().is_some_and(
            |c| c.0.cancel.load(Ordering::Acquire) && c.0.critical.load(Ordering::Acquire) == 0
        ),
        "操作已因应用更新取消；已提交进度保留"
    );
    Ok(())
}
pub fn prepare() -> serde_json::Value {
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    r.preparing = true;
    for task in r.tasks.iter().filter_map(|t| t.upgrade()) {
        if task.cancellable {
            task.cancel.store(true, Ordering::Release);
        }
    }
    drop(r);
    status()
}
pub fn resume() -> serde_json::Value {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .preparing = false;
    status()
}
pub fn status() -> serde_json::Value {
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    r.tasks.retain(|t| t.strong_count() > 0);
    let protected: Vec<_> = r
        .tasks
        .iter()
        .filter_map(|t| t.upgrade())
        .filter(|t| t.protected || t.critical.load(Ordering::Acquire) > 0)
        .map(|t| t.method.clone())
        .collect();
    serde_json::json!({"preparing":r.preparing,"protected":protected,"active":r.tasks.len()})
}

/// Credential rotation may finish its recovery journal even after cancellation.
pub struct Critical(Option<Context>);
pub fn critical() -> Result<Critical> {
    let _registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    check()?;
    let context = context();
    if let Some(c) = &context {
        c.0.critical.fetch_add(1, Ordering::AcqRel);
    }
    Ok(Critical(context))
}
impl Drop for Critical {
    fn drop(&mut self) {
        if let Some(c) = &self.0 {
            c.0.critical.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// Query cancellation is scoped to one caller-supplied id, including queued queries.
pub fn identify_query(id: &str) {
    if let Some(context) = context() {
        *context.0.query_id.lock().unwrap() = id.to_owned();
    }
}
pub fn cancel_query(id: &str) -> serde_json::Value {
    let registry = registry().lock().unwrap();
    let mut count = 0;
    if !id.is_empty() {
        for task in registry.tasks.iter().filter_map(|task| task.upgrade()) {
            if matches!(task.method.as_str(), "dashboard" | "dashboard.summary")
                && *task.query_id.lock().unwrap() == id
            {
                task.query_cancelled.store(true, Ordering::Release);
                task.cancel.store(true, Ordering::Release);
                count += 1;
            }
        }
    }
    serde_json::json!({"cancelled":count})
}
pub(crate) type Progress = std::sync::Arc<Mutex<Box<dyn FnMut(serde_json::Value) + Send>>>;
thread_local! {
    static QUERY_PROGRESS: RefCell<Option<Progress>> = const { RefCell::new(None) };
    /// Callbacks this thread is currently running, innermost last.
    static DELIVERING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}
pub(crate) struct QueryProgress(Option<Progress>);
impl Drop for QueryProgress {
    fn drop(&mut self) {
        QUERY_PROGRESS.with(|p| *p.borrow_mut() = self.0.take());
    }
}
pub(crate) fn query_progress(callback: Progress) -> QueryProgress {
    QueryProgress(QUERY_PROGRESS.with(|p| p.replace(Some(callback))))
}
struct Delivering;
impl Drop for Delivering {
    fn drop(&mut self) {
        DELIVERING.with(|d| d.borrow_mut().pop());
    }
}
/// Every progress event goes through here. A callback may synchronously run
/// engine or store work that reports progress again on the same thread (a slow
/// dashboard read calls `report_query`); its mutex is already held by this
/// thread, so that nested report is dropped instead of deadlocking. Other
/// threads still wait their turn, which keeps delivery order unchanged.
pub(crate) fn deliver(callback: &Progress, value: serde_json::Value) {
    let id = Arc::as_ptr(callback) as *const () as usize;
    if DELIVERING.with(|d| d.borrow().contains(&id)) {
        return;
    }
    // A callback that panicked must not turn later reports into panics.
    let mut callback = callback.lock().unwrap_or_else(|e| e.into_inner());
    DELIVERING.with(|d| d.borrow_mut().push(id));
    let _delivering = Delivering;
    callback(value);
}
pub(crate) fn report_query(done: i64, total: i64) {
    // Clone out of the slot first: the callback may install or restore a slot.
    let Some(callback) = QUERY_PROGRESS.with(|p| p.borrow().clone()) else {
        return;
    };
    deliver(
        &callback,
        serde_json::json!({"kind":"dashboard.progress","stage":format!("dashboard:{done}:{total}"),"done":done,"total":total}),
    );
}

#[cfg(test)]
mod query_tests {
    use super::*;
    #[test]
    fn cancellation_only_affects_the_matching_query() {
        let _scope = enter("dashboard").unwrap();
        identify_query("query-regression");
        assert_eq!(cancel_query("other")["cancelled"], 0);
        assert!(check().is_ok());
        assert_eq!(cancel_query("query-regression")["cancelled"], 1);
        assert!(check().is_err());
    }

    /// Runs `body` on its own thread and fails instead of hanging the suite.
    fn within_timeout<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(body());
        });
        rx.recv_timeout(std::time::Duration::from_secs(20))
            .expect("progress reporting deadlocked when re-entered from its own callback")
    }
    type Seen = Arc<Mutex<Vec<serde_json::Value>>>;
    #[test]
    fn a_callback_that_reports_progress_again_does_not_deadlock() {
        let seen = within_timeout(|| {
            let seen = Seen::default();
            let log = seen.clone();
            let progress: Progress = Arc::new(Mutex::new(Box::new(move |value| {
                log.lock().unwrap().push(value);
                // What a slow dashboard read does on the callback's own thread.
                report_query(99, 100);
            })));
            {
                let _slot = query_progress(progress.clone());
                report_query(1, 3);
                deliver(&progress, serde_json::json!({"stage":"direct"}));
                report_query(2, 3);
            }
            // The slot is restored, so nothing is delivered after the call.
            report_query(3, 3);
            seen
        });
        let seen = seen.lock().unwrap();
        let stages: Vec<_> = seen.iter().map(|v| v["stage"].as_str().unwrap()).collect();
        assert_eq!(stages, ["dashboard:1:3", "direct", "dashboard:2:3"]);
    }
    #[test]
    fn nested_reports_only_skip_the_callback_already_running() {
        let seen = within_timeout(|| {
            let seen = Seen::default();
            let inner_log = seen.clone();
            let inner: Progress = Arc::new(Mutex::new(Box::new(move |value| {
                inner_log.lock().unwrap().push(value);
            })));
            let outer_log = seen.clone();
            let outer: Progress = Arc::new(Mutex::new(Box::new(move |value| {
                outer_log.lock().unwrap().push(value);
                // A different callback installed from inside this one still hears.
                let _slot = query_progress(inner.clone());
                report_query(5, 9);
            })));
            let _slot = query_progress(outer);
            report_query(1, 9);
            report_query(2, 9);
            seen
        });
        let seen = seen.lock().unwrap();
        let stages: Vec<_> = seen.iter().map(|v| v["stage"].as_str().unwrap()).collect();
        assert_eq!(
            stages,
            [
                "dashboard:1:9",
                "dashboard:5:9",
                "dashboard:2:9",
                "dashboard:5:9"
            ]
        );
    }
    #[test]
    fn engine_progress_callbacks_may_report_query_progress() {
        let seen = within_timeout(|| {
            let root = tempfile::tempdir().unwrap();
            let mut engine = crate::Engine::open(root.path()).unwrap();
            let settings = engine.call("settings.get", serde_json::json!({})).unwrap();
            let seen = Seen::default();
            let log = seen.clone();
            engine
                .call_with_progress(
                    "settings.save",
                    settings,
                    Box::new(move |value| {
                        log.lock().unwrap().push(value);
                        report_query(1, 2);
                    }),
                )
                .unwrap();
            seen
        });
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0]["stage"].is_string(), "{seen:?}");
    }
}
