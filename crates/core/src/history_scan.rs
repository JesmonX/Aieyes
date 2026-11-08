//! Local imports run while remote sources are being collected. Each committed
//! source can refresh the UI without waiting for the slowest host in the batch.
use crate::{
    Engine, codex_api, codex_routes, import, models::*, operations, remote_history, runtime,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};

impl Engine {
    pub(crate) fn scan_history(&mut self, params: Value, full: bool) -> Result<Value> {
        if full {
            self.progress("全量重扫");
        }
        let settings = self.store.settings()?;
        let explicit = params["sourceId"].is_string() || params["sourceIds"].is_array();
        let selected: Vec<_> = settings
            .sources
            .iter()
            .filter(|s| {
                settings.source_enabled(s)
                    && s.provider != "deepseek"
                    && s.subscription.is_none()
                    && params["sourceId"].as_str().is_none_or(|id| s.id == id)
                    && params["sourceIds"]
                        .as_array()
                        .is_none_or(|ids| ids.iter().any(|id| id.as_str() == Some(&s.id)))
                    && (explicit
                        || s.host_id.as_ref().is_none_or(|id| {
                            settings
                                .hosts
                                .iter()
                                .find(|h| &h.id == id)
                                .is_none_or(|h| h.enabled)
                        }))
            })
            .collect();
        let remote: Vec<_> = selected
            .iter()
            .copied()
            .filter(|s| s.host_id.is_some())
            .collect();
        let root = self.store.root.clone();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut results = Vec::new();
        std::thread::scope(|scope| -> Result<()> {
            let (tx, rx) = std::sync::mpsc::sync_channel(4);
            for _ in 0..remote.len().min(2) {
                let (tx, remote, settings, next, root) =
                    (tx.clone(), &remote, &settings, &next, &root);
                let operation = operations::context();
                scope.spawn(move || {
                    let _context = operations::inherit(operation);
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(source) = remote.get(index) else {
                            break;
                        };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            operations::check()?;
                            if full {
                                remote_history::rescan(root, settings, source)
                            } else {
                                remote_history::scan(root, settings, source)
                            }
                        }))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("远程同步任务失败")));
                        if tx.send((source.id.clone(), result)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            for source in selected.iter().copied().filter(|s| s.host_id.is_none()) {
                operations::check()?;
                let result = codex_routes::sync(&self.store, source).and_then(|_| {
                    if full {
                        self.progress(&format!("全量重扫 · {}", source.name));
                        import::rescan(&self.store, &settings, source)
                    } else {
                        import::scan(&self.store, &settings, source)
                    }
                });
                results.push(self.finish_history_source(&settings, source, result)?);
            }
            for (id, result) in rx {
                operations::check()?;
                let source = selected
                    .iter()
                    .find(|s| s.id == id)
                    .context("远程读取结果缺失")?;
                results.push(self.finish_history_source(&settings, source, result)?);
            }
            Ok(())
        })?;
        // Completion order is streamed, but the RPC reply retains configuration order.
        results.sort_by_key(|row| {
            selected
                .iter()
                .position(|s| row["id"].as_str() == Some(&s.id))
        });
        Ok(json!(results))
    }

    fn finish_history_source(
        &mut self,
        settings: &Settings,
        source: &Source,
        result: Result<Value>,
    ) -> Result<Value> {
        let result = result.and_then(|value| {
            codex_api::usage::sync(&self.store, settings, source)?;
            Ok(value)
        });
        let row = match result {
            Ok(value) => {
                if value["error"].is_string()
                    || value["failedFiles"].as_u64().unwrap_or(0) > 0
                    || value["invalidRecords"].as_u64().unwrap_or(0) > 0
                    || value["issues"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|i| i["code"] == "invalidRecord")
                {
                    self.store.pause_source_estimates(&source.id)?;
                }
                let mut row = json!({"id":source.id,"name":source.name,"partial":value["partial"],"issues":value["issues"],"result":value});
                if value["error"].is_string() {
                    row["error"] = value["error"].clone();
                }
                row
            }
            Err(error) => {
                let row = json!({"id":source.id,"name":source.name,"error":error.to_string(),"failedAt":now()});
                self.store.source_status(&source.id, &row)?;
                self.store.pause_source_estimates(&source.id)?;
                row
            }
        };
        if let Some(progress) = &mut self.progress {
            progress(
                json!({"kind":"runtime.changed", "revisions":runtime::revisions(&self.store.db)?, "historyCommitted":true}),
            );
        }
        runtime::signal(&self.store.root).wake();
        Ok(row)
    }
}
