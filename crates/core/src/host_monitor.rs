//! One long-lived monitor per host lane. Network work never holds the Engine lock.
use crate::{metrics, models::Host, monitoring, operations, ssh};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

type Sink = Arc<dyn Fn(Value) + Send + Sync>;
type Fetch = Arc<dyn Fn(&Host, bool) -> Result<Value> + Send + Sync>;
struct Entry {
    host: Host,
    fingerprint: String,
    row: Option<Value>,
    previous: Option<Value>,
    next: Instant,
    running: Option<u64>,
}
struct State {
    entries: HashMap<String, Entry>,
    order: Vec<String>,
    jobs: HashMap<u64, (bool, operations::Context)>,
    interval: Duration,
    paused: bool,
    stopped: bool,
    session: String,
    generation: u64,
    version: u64,
    wake: u64,
    sink: Option<Sink>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
struct Job {
    id: u64,
    host: Host,
    previous: Option<Value>,
    context: operations::Context,
}
struct Completion {
    id: u64,
    host: Host,
    result: Result<Value>,
}

pub(crate) struct HostMonitor {
    shared: Arc<Shared>,
    coordinator: Option<thread::JoinHandle<()>>,
    fetch: Fetch,
}
impl Default for HostMonitor {
    fn default() -> Self {
        Self::new(Arc::new(|host, warm| {
            ssh::python(
                host,
                ssh::METRICS_SCRIPT,
                &[serde_json::to_string(&host.metrics)?, warm.to_string()],
            )
        }))
    }
}
impl HostMonitor {
    fn new(fetch: Fetch) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    entries: HashMap::new(),
                    order: vec![],
                    jobs: HashMap::new(),
                    interval: Duration::from_secs(10),
                    paused: true,
                    stopped: false,
                    session: format!(
                        "{}-{}",
                        std::process::id(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ),
                    generation: 0,
                    version: 0,
                    wake: 0,
                    sink: None,
                }),
                changed: Condvar::new(),
            }),
            coordinator: None,
            fetch,
        }
    }
    pub(crate) fn set_sink(&mut self, sink: Sink) {
        self.shared.state.lock().unwrap().sink = Some(sink);
    }
    pub(crate) fn configure(
        &mut self,
        hosts: Vec<Host>,
        interval: Duration,
        paused: bool,
    ) -> Value {
        let mut state = self.shared.state.lock().unwrap();
        let now = Instant::now();
        let ids: Vec<_> = hosts
            .iter()
            .filter(|h| h.enabled)
            .map(|h| h.id.clone())
            .collect();
        if self.coordinator.is_some()
            && state.order == ids
            && state.interval == interval
            && state.paused == paused
            && hosts.iter().filter(|h| h.enabled).all(|h| {
                state.entries.get(&h.id).is_some_and(|entry| {
                    serde_json::to_string(h).is_ok_and(|value| value == entry.fingerprint)
                })
            })
        {
            return snapshot(&state);
        }
        let removed: Vec<_> = state
            .entries
            .keys()
            .filter(|id| !ids.contains(id))
            .cloned()
            .collect();
        for id in removed {
            if let Some(token) = state.entries.remove(&id).and_then(|entry| entry.running)
                && let Some((_, context)) = state.jobs.get(&token)
            {
                context.cancel();
            }
        }
        for host in hosts.into_iter().filter(|h| h.enabled) {
            let fingerprint = serde_json::to_string(&host).unwrap_or_default();
            if state
                .entries
                .get(&host.id)
                .is_none_or(|entry| entry.fingerprint != fingerprint)
            {
                let old = state.entries.remove(&host.id);
                if let Some(token) = old.as_ref().and_then(|entry| entry.running)
                    && let Some((_, context)) = state.jobs.get(&token)
                {
                    context.cancel();
                }
                state.entries.insert(
                    host.id.clone(),
                    Entry {
                        host,
                        fingerprint,
                        row: None,
                        previous: None,
                        next: now,
                        running: None,
                    },
                );
            }
        }
        if interval < state.interval || (state.paused && !paused) {
            for entry in state.entries.values_mut() {
                entry.next = entry.next.min(now);
            }
        }
        state.order = ids;
        state.interval = interval;
        state.paused = paused;
        if paused {
            for (_, context) in state.jobs.values() {
                context.cancel();
            }
        }
        state.wake += 1;
        state.version += 1;
        let result = snapshot(&state);
        drop(state);
        if self.coordinator.is_none() {
            let shared = self.shared.clone();
            let fetch = self.fetch.clone();
            self.coordinator = Some(thread::spawn(move || run(shared, fetch)));
        }
        self.shared.changed.notify_all();
        publish(&self.shared);
        result
    }
    pub(crate) fn refresh(&self, host: Option<&str>) -> Result<Value> {
        let mut state = self.shared.state.lock().unwrap();
        ensure!(
            host.is_none_or(|id| state.entries.contains_key(id)),
            "服务器不存在或监控已暂停"
        );
        for entry in state
            .entries
            .values_mut()
            .filter(|entry| host.is_none_or(|id| entry.host.id == id))
        {
            // An in-flight sample already satisfies a simultaneous refresh.
            if entry.running.is_none() {
                entry.next = Instant::now();
            }
        }
        state.wake += 1;
        state.version += 1;
        let result = snapshot(&state);
        drop(state);
        self.shared.changed.notify_all();
        Ok(result)
    }
    pub(crate) fn configured(&self) -> bool {
        self.coordinator.is_some()
    }
}
impl Drop for HostMonitor {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.stopped = true;
            for (_, context) in state.jobs.values() {
                context.cancel();
            }
            state.wake += 1;
        }
        self.shared.changed.notify_all();
        if let Some(thread) = self.coordinator.take() {
            let _ = thread.join();
        }
    }
}
fn snapshot(state: &State) -> Value {
    let rows: Vec<_> = state
        .order
        .iter()
        .filter_map(|id| state.entries.get(id))
        .map(|entry| {
            entry
                .row
                .clone()
                .unwrap_or_else(|| json!({"id":entry.host.id,"name":entry.host.name}))
        })
        .collect();
    let in_flight: Vec<_> = state
        .order
        .iter()
        .filter(|id| state.entries.get(*id).is_some_and(|e| e.running.is_some()))
        .collect();
    let pending: Vec<_> = state
        .order
        .iter()
        .filter(|id| {
            !state.paused
                && state
                    .entries
                    .get(*id)
                    .is_some_and(|e| e.running.is_none() && e.next <= Instant::now())
        })
        .collect();
    let next = state
        .entries
        .values()
        .filter(|e| e.running.is_none())
        .map(|e| e.next.saturating_duration_since(Instant::now()))
        .min();
    json!({"rows":rows,"inFlight":in_flight,"pending":pending,"sampleSession":state.session,"version":state.version,"paused":state.paused,
        "nextDueAt":next.map(|d| crate::models::now() as f64 + d.as_secs_f64())})
}
fn publish(shared: &Shared) {
    let (sink, value) = {
        let state = shared.state.lock().unwrap();
        (state.sink.clone(), snapshot(&state))
    };
    if let Some(sink) = sink {
        sink(value);
    }
}
fn worker(
    shared: Arc<Shared>,
    input: Arc<Mutex<mpsc::Receiver<Job>>>,
    output: mpsc::Sender<Completion>,
    fetch: Fetch,
    local: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut sampler = monitoring::LocalSampler::default();
        loop {
            let job = { input.lock().unwrap().recv() };
            let Ok(job) = job else { break };
            let _operation = operations::inherit(Some(job.context));
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                operations::check()?;
                if local {
                    Ok(sampler.sample(&job.host.metrics))
                } else {
                    fetch(&job.host, job.previous.is_none())
                }
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("采样任务失败")));
            if output
                .send(Completion {
                    id: job.id,
                    host: job.host,
                    result,
                })
                .is_err()
            {
                break;
            }
            let mut state = shared.state.lock().unwrap();
            state.wake += 1;
            shared.changed.notify_one();
        }
    })
}
fn run(shared: Arc<Shared>, fetch: Fetch) {
    let (remote_tx, remote_rx) = mpsc::channel();
    let remote_rx = Arc::new(Mutex::new(remote_rx));
    let (local_tx, local_rx) = mpsc::channel();
    let local_rx = Arc::new(Mutex::new(local_rx));
    let (done_tx, done_rx) = mpsc::channel();
    let mut workers = vec![worker(
        shared.clone(),
        local_rx,
        done_tx.clone(),
        fetch.clone(),
        true,
    )];
    for _ in 0..crate::host_sampling::CONCURRENCY {
        workers.push(worker(
            shared.clone(),
            remote_rx.clone(),
            done_tx.clone(),
            fetch.clone(),
            false,
        ));
    }
    drop(done_tx);
    loop {
        let mut state = shared.state.lock().unwrap();
        if state.stopped {
            break;
        }
        let mut changed = false;
        while let Ok(done) = done_rx.try_recv() {
            let cancelled = state
                .jobs
                .remove(&done.id)
                .is_some_and(|(_, context)| context.is_cancelled());
            let session = state.session.clone();
            let interval = state.interval;
            if let Some(entry) = state
                .entries
                .get_mut(&done.host.id)
                .filter(|entry| entry.running == Some(done.id))
            {
                entry.running = None;
                entry.next = Instant::now() + interval;
                if cancelled {
                    // Pause/update cancellation is not a host failure. Resume
                    // with a new baseline and keep the last successful display.
                    entry.previous = None;
                    state.version += 1;
                    changed = true;
                    continue;
                }
                let mut row = match done.result {
                    Ok(raw) => {
                        let mut display = metrics::rates(
                            &raw,
                            entry.previous.as_ref().or_else(|| raw.get("previous")),
                        );
                        monitoring::filter_devices(&mut display, &entry.host);
                        entry.previous = Some(raw);
                        json!({"id":entry.host.id,"name":entry.host.name,"sample":display})
                    }
                    Err(error) => {
                        entry.previous = None;
                        json!({"id":entry.host.id,"name":entry.host.name,"sample":entry.row.as_ref().and_then(|r| r.get("sample")),"error":error.to_string()})
                    }
                };
                row["sampleSession"] = json!(session);
                row["sampleVersion"] = json!(done.id);
                entry.row = Some(row);
                state.version += 1;
                changed = true;
            }
        }
        let mut remote_free = crate::host_sampling::CONCURRENCY
            .saturating_sub(state.jobs.values().filter(|(local, _)| !local).count());
        let mut local_free = !state.jobs.values().any(|(local, _)| *local);
        let preparing = operations::is_preparing();
        if !state.paused && !preparing {
            let mut due: Vec<_> = state
                .entries
                .values()
                .filter(|e| e.running.is_none() && e.next <= Instant::now())
                .map(|e| (e.next, e.host.id.clone()))
                .collect();
            due.sort();
            for (_, id) in due {
                let local = id == "local";
                if (local && !local_free) || (!local && remote_free == 0) {
                    continue;
                }
                let Ok(_scope) = operations::enter("hosts.sample") else {
                    break;
                };
                let context = operations::context().unwrap();
                state.generation += 1;
                let token = state.generation;
                let entry = state.entries.get_mut(&id).unwrap();
                entry.running = Some(token);
                let job = Job {
                    id: token,
                    host: entry.host.clone(),
                    previous: entry.previous.clone(),
                    context: context.clone(),
                };
                state.jobs.insert(token, (local, context));
                let _ = if local {
                    local_free = false;
                    local_tx.send(job)
                } else {
                    remote_free -= 1;
                    remote_tx.send(job)
                };
                state.version += 1;
                changed = true;
            }
        }
        let delay = if preparing {
            Duration::from_secs(1)
        } else if state.paused {
            Duration::from_secs(60)
        } else {
            state
                .entries
                .values()
                .filter(|e| {
                    e.running.is_none()
                        && if e.host.id == "local" {
                            local_free
                        } else {
                            remote_free > 0
                        }
                })
                .map(|e| e.next.saturating_duration_since(Instant::now()))
                .min()
                .unwrap_or(Duration::from_secs(60))
        };
        let wake = state.wake;
        if changed {
            drop(state);
            publish(&shared);
            state = shared.state.lock().unwrap();
        }
        let _ = shared
            .changed
            .wait_timeout_while(state, delay, |s| !s.stopped && s.wake == wake);
    }
    drop(remote_tx);
    drop(local_tx);
    for thread in workers {
        let _ = thread.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str) -> Host {
        Host {
            id: id.into(),
            name: id.into(),
            enabled: true,
            metrics: vec!["memory".into()],
            ..Default::default()
        }
    }

    fn wait_snapshot(rx: &mpsc::Receiver<Value>, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let value = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("monitor snapshot timed out");
            if predicate(&value) {
                return value;
            }
        }
    }

    #[test]
    fn unchanged_configuration_does_not_publish_or_advance_the_version() {
        let mut monitor =
            HostMonitor::new(Arc::new(|_, _| panic!("paused monitor must not sample")));
        let (tx, rx) = mpsc::channel();
        monitor.set_sink(Arc::new(move |value| {
            tx.send(value).unwrap();
        }));
        let initial = monitor.configure(vec![host("fast")], Duration::from_secs(10), true);
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)).unwrap(), initial);
        for _ in 0..10 {
            assert_eq!(
                monitor.configure(vec![host("fast")], Duration::from_secs(10), true)["version"],
                initial["version"]
            );
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn changed_host_cancels_the_old_job_and_discards_its_late_success() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let mut monitor = HostMonitor::new(Arc::new(move |host, _| {
            started_tx
                .send((host.target.clone(), operations::context().unwrap()))
                .unwrap();
            if host.target == "old" {
                // Simulate an uninterruptible completion racing a configuration
                // change. A successful response still belongs to the old job.
                let _ = release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3));
            }
            Ok(json!({"fixture":host.target}))
        }));
        let (tx, rx) = mpsc::channel();
        monitor.set_sink(Arc::new(move |value| {
            let _ = tx.send(value);
        }));
        let mut original = host("peer");
        original.target = "old".into();
        monitor.configure(vec![original.clone()], Duration::from_secs(60), false);
        let (target, context) = started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(target, "old");
        original.target = "new".into();
        monitor.configure(vec![original], Duration::from_secs(60), false);
        assert!(context.is_cancelled());
        wait_snapshot(&rx, |value| value["rows"][0]["sample"]["fixture"] == "new");
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !monitor.shared.state.lock().unwrap().jobs.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        let state = monitor.shared.state.lock().unwrap();
        assert!(state.jobs.is_empty());
        assert_eq!(snapshot(&state)["rows"][0]["sample"]["fixture"], "new");
        for value in rx.try_iter() {
            assert_ne!(value["rows"][0]["sample"]["fixture"], "old");
        }
    }

    #[test]
    fn paused_sample_keeps_the_last_success_and_resumes_with_a_new_baseline() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let mut monitor = HostMonitor::new(Arc::new(move |_, warm| {
            let call = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 1 {
                started_tx.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(3);
                while Instant::now() < deadline {
                    operations::check()?;
                    thread::sleep(Duration::from_millis(2));
                }
                panic!("pause did not cancel the sample");
            }
            Ok(json!({"call":call,"warm":warm}))
        }));
        let (tx, rx) = mpsc::channel();
        monitor.set_sink(Arc::new(move |value| {
            let _ = tx.send(value);
        }));
        let hosts = vec![host("peer")];
        monitor.configure(hosts.clone(), Duration::from_secs(60), false);
        let initial = wait_snapshot(&rx, |value| value["rows"][0]["sample"]["call"] == 0);
        monitor.refresh(Some("peer")).unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        monitor.configure(hosts.clone(), Duration::from_secs(60), true);
        let paused = wait_snapshot(&rx, |value| {
            value["paused"] == true && value["inFlight"].as_array().is_some_and(Vec::is_empty)
        });
        assert_eq!(paused["rows"], initial["rows"]);
        monitor.configure(hosts, Duration::from_secs(60), false);
        let resumed = wait_snapshot(&rx, |value| value["rows"][0]["sample"]["call"] == 2);
        assert_eq!(resumed["rows"][0]["sample"]["warm"], true);
    }

    #[test]
    fn cancelled_jobs_count_towards_the_limit_and_do_not_block_local_sampling() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let release = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (r, a, p) = (release.clone(), active.clone(), peak.clone());
        let (started_tx, started_rx) = mpsc::channel();
        let mut monitor = HostMonitor::new(Arc::new(move |host, _| {
            let count = a.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(count, Ordering::SeqCst);
            started_tx
                .send((host.id.clone(), operations::context().unwrap()))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while host.id.starts_with("old")
                && !r.load(Ordering::Acquire)
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(2));
            }
            a.fetch_sub(1, Ordering::SeqCst);
            Ok(json!({"fixture":host.id}))
        }));
        let (tx, rx) = mpsc::channel();
        monitor.set_sink(Arc::new(move |value| {
            let _ = tx.send(value);
        }));
        monitor.configure(
            (0..crate::host_sampling::CONCURRENCY)
                .map(|i| host(&format!("old-{i}")))
                .collect(),
            Duration::from_secs(60),
            false,
        );
        let old: Vec<_> = (0..crate::host_sampling::CONCURRENCY)
            .map(|_| started_rx.recv_timeout(Duration::from_secs(1)).unwrap())
            .collect();
        let mut replacement: Vec<_> = (0..crate::host_sampling::CONCURRENCY)
            .map(|i| host(&format!("new-{i}")))
            .collect();
        replacement.push(host("local"));
        monitor.configure(replacement, Duration::from_secs(60), false);
        assert!(old.iter().all(|(_, context)| context.is_cancelled()));
        let local = wait_snapshot(&rx, |value| {
            value["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == "local" && row["sample"].is_object())
        });
        assert!(
            local["rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["id"] != "local")
                .all(|row| row["sample"].is_null())
        );
        assert!(
            started_rx.try_recv().is_err(),
            "new remote work must wait for cancelled jobs to exit"
        );
        release.store(true, Ordering::Release);
        let complete = wait_snapshot(&rx, |value| {
            value["rows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["sample"].is_object())
        });
        assert!(
            complete["rows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| !row["id"].as_str().unwrap().starts_with("old"))
        );
        assert!(peak.load(Ordering::SeqCst) <= crate::host_sampling::CONCURRENCY);
    }

    #[test]
    fn slow_peer_does_not_block_repeated_fast_samples_or_configuration() {
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (a, p) = (active.clone(), peak.clone());
        let mut monitor = HostMonitor::new(Arc::new(move |host, _| {
            let n = a.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            p.fetch_max(n, std::sync::atomic::Ordering::SeqCst);
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(if host.id == "slow" { 400 } else { 5 })
            {
                if let Err(error) = operations::check() {
                    a.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(2));
            }
            a.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(json!({"timestamp":crate::models::now(),"memory":{"total":1024,"available":512}}))
        }));
        let (tx, rx) = mpsc::channel();
        monitor.set_sink(Arc::new(move |v| {
            let _ = tx.send(v);
        }));
        let hosts = ["fast", "slow"]
            .map(|id| Host {
                id: id.into(),
                enabled: true,
                ..Default::default()
            })
            .to_vec();
        monitor.configure(hosts.clone(), Duration::from_millis(20), false);
        let until = Instant::now() + Duration::from_millis(350);
        let mut versions = std::collections::HashSet::new();
        while Instant::now() < until {
            if let Ok(v) = rx.recv_timeout(Duration::from_millis(30)) {
                for row in v["rows"].as_array().unwrap() {
                    if row["id"] == "fast" && row["sampleVersion"].is_number() {
                        versions.insert(row["sampleVersion"].as_u64().unwrap());
                    }
                }
            }
        }
        assert!(
            versions.len() >= 3,
            "Fast host must continue while slow host is still sampling: {versions:?}"
        );
        let started = Instant::now();
        monitor.configure(hosts, Duration::from_millis(20), true);
        assert!(started.elapsed() < Duration::from_millis(50));
        assert!(
            peak.load(std::sync::atomic::Ordering::SeqCst) <= crate::host_sampling::CONCURRENCY
        );
        drop(monitor);
    }
}
