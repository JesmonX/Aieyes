//! Update coordination is independent of engine/SQLite locks.
use anyhow::{Result, ensure};
use std::{cell::RefCell, sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, AtomicUsize, Ordering}}};

#[derive(Default)]
struct Registry { preparing: bool, tasks: Vec<std::sync::Weak<Task>> }
struct Task { method: String, cancel: AtomicBool, critical: AtomicUsize, protected: bool }
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
thread_local! { static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) }; }
fn registry() -> &'static Mutex<Registry> { REGISTRY.get_or_init(Default::default) }
#[derive(Clone)]
pub struct Context(Arc<Task>);
pub struct Scope { previous: Option<Context> }
impl Drop for Scope { fn drop(&mut self) { CURRENT.with(|v| *v.borrow_mut() = self.previous.take()); } }
pub fn context() -> Option<Context> { CURRENT.with(|v| v.borrow().clone()) }
pub fn inherit(context: Option<Context>) -> Scope {
    Scope { previous: CURRENT.with(|v| v.replace(context)) }
}
pub fn enter(method: &str) -> Result<Scope> {
    if context().is_some() { check()?; return Ok(inherit(context())); }
    let behavior = crate::contracts::method(method).map_or("finish", |m| m.update_behavior);
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    ensure!(!r.preparing || behavior == "observe", "正在准备更新，请稍候");
    let task = Arc::new(Task { method: method.into(), cancel: AtomicBool::new(false), critical: AtomicUsize::new(0), protected: behavior != "cancel" });
    r.tasks.retain(|t| t.strong_count() > 0);
    r.tasks.push(Arc::downgrade(&task));
    Ok(inherit(Some(Context(task))))
}
pub fn check() -> Result<()> {
    ensure!(!context().is_some_and(|c| c.0.cancel.load(Ordering::Acquire) && c.0.critical.load(Ordering::Acquire) == 0), "操作已因应用更新取消；已提交进度保留");
    Ok(())
}
pub fn prepare() -> serde_json::Value {
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    r.preparing = true;
    for task in r.tasks.iter().filter_map(|t| t.upgrade()) {
        if !task.protected { task.cancel.store(true, Ordering::Release); }
    }
    drop(r); status()
}
pub fn resume() -> serde_json::Value {
    registry().lock().unwrap_or_else(|e| e.into_inner()).preparing = false;
    status()
}
pub fn status() -> serde_json::Value {
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    r.tasks.retain(|t| t.strong_count() > 0);
    let protected: Vec<_> = r.tasks.iter().filter_map(|t| t.upgrade()).filter(|t| t.protected || t.critical.load(Ordering::Acquire) > 0).map(|t| t.method.clone()).collect();
    serde_json::json!({"preparing":r.preparing,"protected":protected,"active":r.tasks.len()})
}

/// Credential rotation may finish its recovery journal even after cancellation.
pub struct Critical(Option<Context>);
pub fn critical() -> Result<Critical> {
    let _registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    check()?;
    let context = context();
    if let Some(c) = &context { c.0.critical.fetch_add(1, Ordering::AcqRel); }
    Ok(Critical(context))
}
impl Drop for Critical { fn drop(&mut self) { if let Some(c) = &self.0 { c.0.critical.fetch_sub(1, Ordering::AcqRel); } } }
