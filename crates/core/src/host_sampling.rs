//! Bounded host collection. Publish completed hosts without waiting for a slow peer.
use crate::{Engine, metrics, models::Host, monitoring, ssh};
use anyhow::Result;
use serde_json::{Value, json};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc::sync_channel,
};

pub(crate) const CONCURRENCY: usize = 3;

pub(crate) fn collect<F, G>(hosts: &[Host], fetch: F, mut receive: G)
where
    F: Fn(&Host) -> Result<Value> + Sync,
    G: FnMut(usize, Result<Value>),
{
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let (tx, rx) = sync_channel(4);
        let next = &next;
        let fetch = &fetch;
        for _ in 0..hosts.len().min(CONCURRENCY) {
            let tx = tx.clone();
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(host) = hosts.get(index) else { break };
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fetch(host)))
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("采样任务失败")));
                    if tx.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for (index, result) in rx {
            receive(index, result)
        }
    });
}

impl Engine {
    pub(crate) fn sample_hosts(&mut self, params: Value) -> Result<Value> {
        let settings = self.store.settings()?;
        let active: std::collections::HashSet<_> = settings
            .monitored_hosts()
            .into_iter()
            .filter(|h| h.enabled)
            .map(|h| h.id)
            .collect();
        self.previous_metrics.retain(|id, _| active.contains(id));
        let selected: Vec<_> = settings
            .monitored_hosts()
            .into_iter()
            .filter(|h| h.enabled && params["hostId"].as_str().is_none_or(|id| h.id == id))
            .collect();
        self.host_generation += 1;
        let generation = self.host_generation;
        let batch = format!("{}:{generation}", self.host_session);
        let stream = params["stream"].as_bool().unwrap_or(false);
        let mut results = std::collections::HashMap::new();
        let mut accept = |this: &mut Self, host: &Host, sample: Result<Value>| {
            let mut row = match sample {
                Ok(raw) => {
                    let mut display = metrics::rates(
                        &raw,
                        this.previous_metrics
                            .get(&host.id)
                            .or_else(|| raw.get("previous")),
                    );
                    this.previous_metrics.insert(host.id.clone(), raw);
                    monitoring::filter_devices(&mut display, host);
                    json!({"id":host.id,"name":host.name,"sample":display})
                }
                Err(error) => {
                    this.previous_metrics.remove(&host.id);
                    json!({"id":host.id,"name":host.name,"error":error.to_string()})
                }
            };
            if stream {
                row["sampleSession"] = json!(this.host_session);
                row["sampleVersion"] = json!(generation);
                if let Some(progress) = &mut this.progress {
                    progress(json!({"kind":"hosts.sample","batchId":batch,"row":row}));
                }
            }
            results.insert(host.id.clone(), row);
        };
        if let Some(host) = selected.iter().find(|h| h.id == "local") {
            let local = self.local_metrics.sample(&host.metrics);
            accept(self, host, Ok(local));
        }
        let remote: Vec<_> = selected
            .iter()
            .filter(|h| h.id != "local")
            .cloned()
            .collect();
        let warm: std::collections::HashSet<_> = remote
            .iter()
            .filter(|h| !self.previous_metrics.contains_key(&h.id))
            .map(|h| h.id.clone())
            .collect();
        collect(
            &remote,
            |host| {
                ssh::python(
                    host,
                    ssh::METRICS_SCRIPT,
                    &[
                        serde_json::to_string(&host.metrics)?,
                        warm.contains(&host.id).to_string(),
                    ],
                )
            },
            |index, value| accept(self, &remote[index], value),
        );
        let rows: Vec<_> = selected
            .iter()
            .filter_map(|h| results.remove(&h.id))
            .collect();
        Ok(if stream {
            json!({"batchId":batch,"rows":rows})
        } else {
            json!(rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrency_is_bounded_and_completion_is_not_in_host_order() {
        let hosts: Vec<_> = (0..12)
            .map(|i| Host {
                id: i.to_string(),
                ..Default::default()
            })
            .collect();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let mut order = vec![];
        collect(
            &hosts,
            |h| {
                let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(if h.id == "0" {
                    100
                } else {
                    2
                }));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(json!({}))
            },
            |i, _| order.push(i),
        );
        assert_eq!(order.len(), 12);
        assert_ne!(order[0], 0);
        assert!(peak.load(Ordering::SeqCst) <= CONCURRENCY);
    }
}
