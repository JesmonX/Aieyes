use super::{config, configuration, control, db, feishu};
use crate::{file_lock::FileLock, store::Store};
use anyhow::Result;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Frontends own this service. A process-shared lock permits one consumer per data directory.
pub struct Service {
    stopped: Arc<AtomicBool>,
    signal: Arc<crate::runtime::Signal>,
}
impl Service {
    pub fn start(root: &Path) -> Result<Option<Self>> {
        let file = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("channel-service.lock"))?;
        let lock = match FileLock::try_exclusive(file) {
            Ok(lock) => Arc::new(lock),
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let stopped = Arc::new(AtomicBool::new(false));
        let signal = crate::runtime::signal(root);
        // Fail synchronously if a worker cannot open its database.
        let receiver = Store::open(root)?;
        let dispatcher = Store::open(root)?;
        receiver.db.busy_timeout(Duration::from_millis(1200))?;
        dispatcher.db.busy_timeout(Duration::from_millis(1200))?;
        {
            let signal = signal.clone();
            let stopped = stopped.clone();
            let lock = lock.clone();
            std::thread::Builder::new()
                .name("aieyes-feishu-receive".into())
                .spawn(move || {
                    let _lock = lock;
                    let store = receiver;
                    while !stopped.load(Ordering::Acquire) {
                        let generation = signal.generation();
                        let mut wait = Duration::from_secs(2);
                        if let Ok(c) = configuration(&store.db)
                            && c.enabled
                            && !c.secret_ref.is_empty()
                            && let Ok(settings) = store.settings()
                        {
                            let _ = config::connection(&store.db, &c.epoch, "connecting", None);
                            match feishu::connection(&store, &c, &settings.proxy, &stopped) {
                                Ok(seconds) => wait = Duration::from_secs(seconds),
                                Err(_) => {
                                    let _ = config::connection(
                                        &store.db,
                                        &c.epoch,
                                        "reconnecting",
                                        Some("飞书连接失败，请检查应用凭据、事件订阅和网络"),
                                    );
                                    wait = Duration::from_secs(15);
                                }
                            }
                        }
                        if !stopped.load(Ordering::Acquire) {
                            signal.wait(generation, wait);
                        }
                    }
                })?;
        }
        {
            let signal = signal.clone();
            let worker_stop = stopped.clone();
            let spawned = std::thread::Builder::new()
                .name("aieyes-channel-control".into())
                .spawn(move || {
                    let _lock = lock;
                    let store = dispatcher;
                    if control::recover(&store).is_err() {
                        let _ = config::worker_error(&store, "无法恢复渠道指令，请检查本机数据库");
                    }
                    let mut api = None;
                    let mut api_key = String::new();
                    let mut last_confirm = Instant::now();
                    let mut last_cleanup = Instant::now() - Duration::from_secs(60);
                    while !worker_stop.load(Ordering::Acquire) {
                        let generation = signal.generation();
                        let work = (|| -> Result<()> {
                            if crate::operations::status()["preparing"] != true
                                && configuration(&store.db)?.enabled
                            {
                                db::process_inbox(&store)?;
                                // Update coordination waits for a concrete submission to finish.
                                if let Ok(_operation) =
                                    crate::operations::enter("channels.dispatch")
                                {
                                    control::process_command(&store)?;
                                    control::process_event(&store)?;
                                    if last_confirm.elapsed() >= Duration::from_secs(5) {
                                        control::confirm(&store)?;
                                        last_confirm = Instant::now();
                                    }
                                }
                                let current = configuration(&store.db)?;
                                if current.owner.is_some() {
                                    let settings = store.settings()?;
                                    let key = format!(
                                        "{}:{}",
                                        current.epoch,
                                        serde_json::to_string(&settings.proxy).unwrap_or_default()
                                    );
                                    if key != api_key {
                                        api = None;
                                        api_key = key;
                                    }
                                    if api.is_none() {
                                        api = Some(feishu::Api::new(&current, &settings.proxy)?);
                                    }
                                    if let Some(api) = api.as_mut() {
                                        feishu::deliver(&store, &current, api)?;
                                    }
                                }
                            }
                            if last_cleanup.elapsed() >= Duration::from_secs(60) {
                                db::cleanup(&store.db)?;
                                last_cleanup = Instant::now();
                            }
                            Ok(())
                        })();
                        if work.is_err() {
                            let _ = config::worker_error(
                                &store,
                                "渠道处理暂时失败，请检查系统凭据库和本机数据库",
                            );
                        } else {
                            let _ = store
                                .db
                                .execute("DELETE FROM kv WHERE key='channel.workerError'", []);
                        }
                        if !worker_stop.load(Ordering::Acquire) {
                            signal.wait(generation, Duration::from_millis(500));
                        }
                    }
                });
            if let Err(error) = spawned {
                stopped.store(true, Ordering::Release);
                return Err(error.into());
            }
        }
        Ok(Some(Self { stopped, signal }))
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.signal.wake();
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}
