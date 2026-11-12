//! Outbound events on Windows and Linux: webhook delivery and server alerts.
//!
//! Webhook events queue in the core (quota resets, session transitions, server alerts); this
//! worker delivers them when the `webhooks` revision changes and retries every 60 s while a
//! delivery failed or events are pending. Server alerts are read from the hosts lane, whose
//! monitor holds the minute history, and raise a system notification when 服务器告警 is on.
use aieyes_core::{Engine, host_history::AlertEvent, models::now, webhooks::Kind};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};

const RETRY_SECONDS: i64 = 60;
const ALERT_SECONDS: u64 = 15;

pub struct Outbound {
    stopped: Arc<AtomicBool>,
}
impl Outbound {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

/// When to deliver: a new revision, or the retry time of an unfinished delivery.
#[derive(Debug, Default)]
struct Delivery {
    revision: Value,
    retry_at: Option<i64>,
}
impl Delivery {
    fn due(&self, revision: &Value, at: i64) -> bool {
        *revision != self.revision || self.retry_at.is_some_and(|retry| at >= retry)
    }
    /// Records a dispatch result; `revision` is read after the dispatch so its own writes
    /// do not count as new events.
    fn finished(&mut self, revision: Value, result: Result<&Value, ()>, at: i64) {
        self.revision = revision;
        let unfinished = match result {
            Ok(value) => {
                value["failed"].as_i64().unwrap_or(0) > 0
                    || value["pending"].as_i64().unwrap_or(0) > 0
            }
            Err(()) => true,
        };
        self.retry_at = unfinished.then_some(at + RETRY_SECONDS);
    }
    fn wait(&self, at: i64) -> Duration {
        let seconds = self.retry_at.map_or(15, |retry| (retry - at).clamp(1, 15));
        Duration::from_secs(seconds as u64)
    }
}

/// The same title and body as macOS and the server-alert webhook.
fn alert_content(event: &AlertEvent) -> (String, String) {
    let title = if event.metric == "memory" {
        format!("{} 内存使用率持续偏高", event.name)
    } else {
        format!("{} CPU 使用率持续偏高", event.name)
    };
    let body = format!(
        "当前 {:.0}%，已持续 {} 分钟",
        event.value,
        event.duration_seconds / 60
    );
    (title, body)
}

pub fn start(app: &AppHandle, root: &Path) {
    let stopped = Arc::new(AtomicBool::new(false));
    app.manage(Outbound {
        stopped: stopped.clone(),
    });
    {
        let root = root.to_owned();
        let stopped = stopped.clone();
        std::thread::spawn(move || {
            let signal = aieyes_core::runtime::signal(&root);
            let mut engine = None;
            let mut delivery = Delivery::default();
            while !stopped.load(Ordering::Acquire) {
                let generation = signal.generation();
                if engine.is_none() {
                    engine = Engine::open(&root).ok();
                }
                if let Some(engine) = engine.as_mut()
                    && aieyes_core::operations::status()["preparing"] != true
                    && let Ok(revisions) = aieyes_core::runtime::revisions(&engine.store.db)
                    && delivery.due(&revisions["webhooks"], now())
                {
                    let result = engine.call("webhooks.dispatch", json!({}));
                    let after = aieyes_core::runtime::revisions(&engine.store.db)
                        .map(|r| r["webhooks"].clone())
                        .unwrap_or(revisions["webhooks"].clone());
                    delivery.finished(after, result.as_ref().map_err(|_| ()), now());
                }
                if !stopped.load(Ordering::Acquire) {
                    signal.wait(generation, delivery.wait(now()));
                }
            }
        });
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let since = now();
        let mut asked = false;
        while !stopped.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_secs(ALERT_SECONDS));
            if stopped.load(Ordering::Acquire)
                || aieyes_core::operations::status()["preparing"] == true
            {
                continue;
            }
            let Some(shared) = app.try_state::<crate::Shared>() else {
                continue;
            };
            let Ok(mut engine) = shared.1.lock() else {
                continue;
            };
            let Ok(settings) = engine.store.settings() else {
                continue;
            };
            let notify = settings.server_alerts.enabled;
            if !notify && !settings.webhooks.allows(Kind::ServerAlert) {
                continue;
            }
            // Taking alerts also queues their webhooks (the core flushes them on each call).
            let events = engine
                .call("hostAlerts.take", json!({"since": since}))
                .ok()
                .and_then(|value| serde_json::from_value::<Vec<AlertEvent>>(value).ok())
                .unwrap_or_default();
            drop(engine);
            if !notify {
                continue;
            }
            if !asked {
                asked = true;
                crate::notifications::report(&app, crate::notifications::request(&app));
            }
            for event in events {
                let (title, body) = alert_content(&event);
                crate::notifications::report(&app, crate::notifications::send(&app, &title, &body));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_follows_the_revision_and_retries_unfinished_work() {
        let mut delivery = Delivery::default();
        assert!(delivery.due(&json!(1), 100), "a new revision delivers");
        delivery.finished(
            json!(2),
            Ok(&json!({"delivered":1,"failed":0,"pending":0})),
            100,
        );
        assert!(
            !delivery.due(&json!(2), 100),
            "the dispatch's own write is not new work"
        );
        assert_eq!(delivery.retry_at, None);
        assert_eq!(delivery.wait(100), Duration::from_secs(15));
        delivery.finished(
            json!(2),
            Ok(&json!({"delivered":0,"failed":1,"pending":1})),
            100,
        );
        assert!(!delivery.due(&json!(2), 159));
        assert!(
            delivery.due(&json!(2), 160),
            "a failed delivery retries after 60 s"
        );
        assert_eq!(delivery.wait(150), Duration::from_secs(10));
        delivery.finished(json!(2), Err(()), 200);
        assert_eq!(delivery.retry_at, Some(260));
    }

    #[test]
    fn alerts_use_the_shared_copy() {
        let event = AlertEvent {
            id: "a".into(),
            host_id: "h".into(),
            name: "build-01".into(),
            metric: "memory".into(),
            value: 93.4,
            threshold: 90.0,
            duration_seconds: 300,
            detected_at: 1,
        };
        assert_eq!(
            alert_content(&event),
            (
                "build-01 内存使用率持续偏高".to_string(),
                "当前 93%，已持续 5 分钟".to_string()
            )
        );
        let cpu = AlertEvent {
            metric: "cpu".into(),
            ..event
        };
        assert_eq!(alert_content(&cpu).0, "build-01 CPU 使用率持续偏高");
    }
}
