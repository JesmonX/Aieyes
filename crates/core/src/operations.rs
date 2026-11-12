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
type Progress = std::sync::Arc<Mutex<Box<dyn FnMut(serde_json::Value) + Send>>>;
thread_local! { static QUERY_PROGRESS: RefCell<Option<Progress>> = const { RefCell::new(None) }; }
pub(crate) struct QueryProgress(Option<Progress>);
impl Drop for QueryProgress {
    fn drop(&mut self) {
        QUERY_PROGRESS.with(|p| *p.borrow_mut() = self.0.take());
    }
}
pub(crate) fn query_progress(callback: Progress) -> QueryProgress {
    QueryProgress(QUERY_PROGRESS.with(|p| p.replace(Some(callback))))
}
pub(crate) fn report_query(done: i64, total: i64) {
    QUERY_PROGRESS.with(|p| { if let Some(callback) = p.borrow().as_ref() {
        callback.lock().unwrap()(serde_json::json!({"kind":"dashboard.progress","stage":format!("dashboard:{done}:{total}"),"done":done,"total":total}));
    }});
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
}
