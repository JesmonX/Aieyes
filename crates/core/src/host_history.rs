//! Recent per-host readings kept in memory: minute rollups for sparklines and
//! sustained-threshold alerts. Nothing here is persisted; a restart starts over.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, mpsc},
};

/// Minutes kept per host; sparklines show the tail of this window.
pub const MINUTES: usize = 180;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlertPreferences {
    pub enabled: bool,
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub duration_seconds: u64,
}
impl Default for AlertPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            cpu_percent: 90.0,
            memory_percent: 90.0,
            duration_seconds: 300,
        }
    }
}

pub fn validate(preferences: &AlertPreferences) -> Result<()> {
    for value in [preferences.cpu_percent, preferences.memory_percent] {
        ensure!(
            value.is_finite() && (1.0..=100.0).contains(&value),
            "服务器告警阈值范围为 1–100%"
        );
    }
    ensure!(
        (60..=86_400).contains(&preferences.duration_seconds),
        "服务器告警持续时间范围为 1–1440 分钟"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub id: String,
    pub host_id: String,
    pub name: String,
    /// "cpu" or "memory".
    pub metric: String,
    pub value: f64,
    pub threshold: f64,
    pub duration_seconds: u64,
    pub detected_at: i64,
}

#[derive(Debug, Clone, Default, Copy)]
struct Reading {
    cpu: Option<f64>,
    memory: Option<f64>,
    gpu: Option<f64>,
}

fn within_unit(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 100.0))
}

fn reading(sample: &Value) -> Reading {
    let cpu = within_unit(
        sample["cpu"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"] == "cpu"))
            .and_then(|row| row["utilization"].as_f64()),
    );
    let memory = sample["memory"].as_object().and_then(|memory| {
        let total = memory.get("total").and_then(Value::as_f64)?;
        if !total.is_finite() || total <= 0.0 {
            return None;
        }
        let used = memory.get("used").and_then(Value::as_f64).or_else(|| {
            memory
                .get("available")
                .and_then(Value::as_f64)
                .map(|available| total - available)
        })?;
        within_unit(Some(used / total * 100.0))
    });
    let gpu = within_unit(sample["gpu"].as_array().and_then(|rows| {
        rows.iter()
            .filter_map(|row| row["utilization"].as_f64())
            .reduce(f64::max)
    }));
    Reading { cpu, memory, gpu }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Minute {
    /// Start of the minute, epoch seconds.
    pub t: i64,
    pub cpu: Option<f64>,
    pub cpu_max: Option<f64>,
    pub memory: Option<f64>,
    pub gpu: Option<f64>,
}

#[derive(Default, Clone, Copy)]
struct Accum {
    samples: u32,
    cpu_sum: f64,
    cpu_n: u32,
    cpu_max: f64,
    memory_sum: f64,
    memory_n: u32,
    gpu_max: Option<f64>,
}
impl Accum {
    fn add(&mut self, reading: Reading) {
        self.samples += 1;
        if let Some(cpu) = reading.cpu {
            self.cpu_sum += cpu;
            self.cpu_n += 1;
            self.cpu_max = self.cpu_max.max(cpu);
        }
        if let Some(memory) = reading.memory {
            self.memory_sum += memory;
            self.memory_n += 1;
        }
        if let Some(gpu) = reading.gpu {
            self.gpu_max = Some(self.gpu_max.map_or(gpu, |current: f64| current.max(gpu)));
        }
    }
    fn minute(&self, t: i64) -> Minute {
        Minute {
            t,
            cpu: (self.cpu_n > 0).then(|| self.cpu_sum / f64::from(self.cpu_n)),
            cpu_max: (self.cpu_n > 0).then_some(self.cpu_max),
            memory: (self.memory_n > 0).then(|| self.memory_sum / f64::from(self.memory_n)),
            gpu: self.gpu_max,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Above {
    since: Option<i64>,
    fired: bool,
}

#[derive(Default)]
struct Series {
    name: String,
    minutes: VecDeque<Minute>,
    current: Option<(i64, Accum)>,
    cpu: Above,
    memory: Above,
    last_seen: Option<i64>,
}

pub struct HostHistory {
    hosts: HashMap<String, Series>,
    max_gap_seconds: i64,
}
impl Default for HostHistory {
    fn default() -> Self {
        Self {
            hosts: HashMap::new(),
            max_gap_seconds: 150,
        }
    }
}

impl HostHistory {
    fn close_minute(series: &mut Series) {
        if let Some((t, accum)) = series.current.take()
            && accum.samples > 0
        {
            series.minutes.push_back(accum.minute(t));
            while series.minutes.len() > MINUTES {
                series.minutes.pop_front();
            }
        }
    }

    fn evaluate(
        above: &mut Above,
        value: Option<f64>,
        threshold: f64,
        duration: i64,
        stamp: i64,
    ) -> bool {
        let Some(value) = value else {
            *above = Above::default();
            return false;
        };
        if value < threshold {
            *above = Above::default();
            return false;
        }
        let since = above.since.get_or_insert(stamp);
        if !above.fired && stamp - *since >= duration {
            above.fired = true;
            return true;
        }
        false
    }

    /// Fold one sample into the rollup and produce alert events that just fired.
    pub fn ingest(
        &mut self,
        host_id: &str,
        name: &str,
        sample: &Value,
        stamp: i64,
        preferences: &AlertPreferences,
        active: bool,
    ) -> Vec<AlertEvent> {
        let reading = reading(sample);
        let minute = stamp.div_euclid(60) * 60;
        let series = self.hosts.entry(host_id.to_owned()).or_default();
        if series
            .last_seen
            .is_some_and(|previous| stamp < previous || stamp - previous > self.max_gap_seconds)
        {
            series.cpu = Above::default();
            series.memory = Above::default();
        }
        series.last_seen = Some(stamp);
        series.name = name.to_owned();
        if series.current.as_ref().is_some_and(|(t, _)| *t != minute) {
            Self::close_minute(series);
        }
        series
            .current
            .get_or_insert_with(|| (minute, Accum::default()))
            .1
            .add(reading);
        let mut events = Vec::new();
        if !active {
            series.cpu = Above::default();
            series.memory = Above::default();
            return events;
        }
        let duration = i64::try_from(preferences.duration_seconds).unwrap_or(i64::MAX);
        let Series { cpu, memory, .. } = series;
        for (metric, above, value, threshold) in [
            ("cpu", cpu, reading.cpu, preferences.cpu_percent),
            ("memory", memory, reading.memory, preferences.memory_percent),
        ] {
            if Self::evaluate(above, value, threshold, duration, stamp)
                && let Some(value) = value
            {
                events.push(AlertEvent {
                    id: crate::codex_auth::files::new_id(),
                    host_id: host_id.to_owned(),
                    name: name.to_owned(),
                    metric: metric.into(),
                    value,
                    threshold,
                    duration_seconds: preferences.duration_seconds,
                    detected_at: stamp,
                });
            }
        }
        events
    }

    pub fn retain(&mut self, ids: &HashSet<String>) {
        self.hosts.retain(|id, _| ids.contains(id));
    }

    pub fn snapshot(&self, host: Option<&str>) -> Value {
        let rows: Vec<_> = self
            .hosts
            .iter()
            .filter(|(id, _)| host.is_none_or(|wanted| wanted == id.as_str()))
            .map(|(id, series)| {
                let mut minutes: Vec<Minute> = series.minutes.iter().cloned().collect();
                if let Some((t, accum)) = &series.current
                    && accum.samples > 0
                {
                    minutes.push(accum.minute(*t));
                }
                json!({"id":id,"name":series.name,"minutes":minutes})
            })
            .collect();
        json!({"hosts":rows})
    }
}

struct ObserverState {
    history: HostHistory,
    alerts: Vec<AlertEvent>,
    preferences: AlertPreferences,
    webhook: bool,
}

/// Shared between the background monitor threads and the engine's RPC handlers.
pub struct HostObserver {
    state: Mutex<ObserverState>,
    webhook: Mutex<Option<mpsc::Sender<AlertEvent>>>,
}
impl HostObserver {
    pub fn new() -> (Arc<Self>, mpsc::Receiver<AlertEvent>) {
        let (sender, receiver) = mpsc::channel();
        let observer = Arc::new(Self {
            state: Mutex::new(ObserverState {
                history: HostHistory::default(),
                alerts: Vec::new(),
                preferences: AlertPreferences::default(),
                webhook: false,
            }),
            webhook: Mutex::new(Some(sender)),
        });
        (observer, receiver)
    }

    /// Mirror the current settings; called from every hosts call so prefs stay fresh.
    pub fn configure(&self, preferences: &AlertPreferences, webhook: bool) {
        let mut state = self.state.lock().unwrap();
        if state.preferences != *preferences || state.webhook != webhook {
            for series in state.history.hosts.values_mut() {
                series.cpu = Above::default();
                series.memory = Above::default();
            }
        }
        state.preferences = preferences.clone();
        state.webhook = webhook;
    }

    pub fn pause(&self) {
        let mut state = self.state.lock().unwrap();
        for series in state.history.hosts.values_mut() {
            series.cpu = Above::default();
            series.memory = Above::default();
            series.last_seen = None;
        }
    }
    pub fn set_interval(&self, seconds: u64) {
        self.state.lock().unwrap().history.max_gap_seconds = (seconds.saturating_mul(5) / 2)
            .max(150)
            .min(i64::MAX as u64)
            as i64;
    }

    pub fn observe(&self, host_id: &str, name: &str, sample: &Value) {
        self.observe_at(host_id, name, sample, crate::models::now());
    }

    fn observe_at(&self, host_id: &str, name: &str, sample: &Value, stamp: i64) {
        let mut state = self.state.lock().unwrap();
        let preferences = state.preferences.clone();
        let active = preferences.enabled || state.webhook;
        let events = state
            .history
            .ingest(host_id, name, sample, stamp, &preferences, active);
        if events.is_empty() {
            return;
        }
        if preferences.enabled {
            state.alerts.extend(events.iter().cloned());
            if state.alerts.len() > 50 {
                let excess = state.alerts.len() - 50;
                state.alerts.drain(..excess);
            }
        }
        if state.webhook
            && let Some(sender) = self.webhook.lock().unwrap().as_ref()
        {
            for event in events {
                let _ = sender.send(event);
            }
        }
    }

    pub fn retain(&self, ids: &HashSet<String>) {
        self.state.lock().unwrap().history.retain(ids);
    }

    /// Alerts detected at or after `since`; always clears the queue, like reset notifications.
    pub fn take(&self, since: i64) -> Vec<AlertEvent> {
        let mut state = self.state.lock().unwrap();
        let alerts = std::mem::take(&mut state.alerts);
        alerts
            .into_iter()
            .filter(|event| event.detected_at >= since)
            .collect()
    }

    pub fn history(&self, host: Option<&str>) -> Value {
        self.state.lock().unwrap().history.snapshot(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cpu: f64, used: f64, total: f64) -> Value {
        json!({"cpu":[{"id":"cpu","utilization":cpu}],"memory":{"total":total,"used":used}})
    }

    fn preferences(threshold: f64, duration: u64) -> AlertPreferences {
        AlertPreferences {
            enabled: true,
            cpu_percent: threshold,
            memory_percent: threshold,
            duration_seconds: duration,
        }
    }

    #[test]
    fn minutes_roll_up_averages_and_maxima() {
        let mut history = HostHistory::default();
        let prefs = AlertPreferences {
            enabled: false,
            ..Default::default()
        };
        // 1_020 and 1_050 share a minute; 1_100 opens the next one.
        history.ingest(
            "h",
            "Host",
            &sample(10.0, 50.0, 100.0),
            1_020,
            &prefs,
            false,
        );
        history.ingest(
            "h",
            "Host",
            &sample(30.0, 60.0, 100.0),
            1_050,
            &prefs,
            false,
        );
        history.ingest(
            "h",
            "Host",
            &sample(20.0, 70.0, 100.0),
            1_100,
            &prefs,
            false,
        );
        let value = history.snapshot(None);
        let minutes = &value["hosts"][0]["minutes"];
        assert_eq!(minutes.as_array().unwrap().len(), 2);
        assert_eq!(minutes[0]["cpu"], 20.0);
        assert_eq!(minutes[0]["cpuMax"], 30.0);
        assert_eq!(minutes[0]["memory"], 55.0);
        assert_eq!(minutes[1]["cpu"], 20.0);
    }

    #[test]
    fn alerts_fire_once_per_episode_after_the_duration() {
        let mut history = HostHistory {
            max_gap_seconds: 1000,
            ..Default::default()
        };
        let prefs = preferences(90.0, 300);
        assert!(
            history
                .ingest("h", "Host", &sample(95.0, 10.0, 100.0), 1_000, &prefs, true)
                .is_empty()
        );
        assert!(
            history
                .ingest("h", "Host", &sample(95.0, 10.0, 100.0), 1_200, &prefs, true)
                .is_empty()
        );
        let events = history.ingest("h", "Host", &sample(95.0, 10.0, 100.0), 1_300, &prefs, true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].metric, "cpu");
        assert_eq!(events[0].value, 95.0);
        // Still high: no second alert until the value drops and rises again.
        assert!(
            history
                .ingest("h", "Host", &sample(99.0, 10.0, 100.0), 2_000, &prefs, true)
                .is_empty()
        );
        assert!(
            history
                .ingest("h", "Host", &sample(40.0, 10.0, 100.0), 2_100, &prefs, true)
                .is_empty()
        );
        assert!(
            history
                .ingest("h", "Host", &sample(95.0, 10.0, 100.0), 2_200, &prefs, true)
                .is_empty()
        );
        let again = history.ingest("h", "Host", &sample(95.0, 10.0, 100.0), 2_600, &prefs, true);
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn gaps_and_missing_readings_restart_sustained_alerts() {
        let mut history = HostHistory::default();
        let prefs = preferences(90.0, 300);
        for stamp in [1000, 4600, 4660, 4720, 4780, 4840] {
            assert!(
                history
                    .ingest("h", "Host", &sample(95.0, 10.0, 100.0), stamp, &prefs, true)
                    .is_empty()
            );
        }
        assert_eq!(
            history
                .ingest("h", "Host", &sample(95.0, 10.0, 100.0), 4900, &prefs, true)
                .len(),
            1
        );
        history.ingest("h", "Host", &Value::Null, 4960, &prefs, true);
        assert!(
            history
                .ingest("h", "Host", &sample(95.0, 10.0, 100.0), 5020, &prefs, true)
                .is_empty()
        );
    }

    #[test]
    fn inactive_preferences_only_record_history() {
        let mut history = HostHistory::default();
        let prefs = preferences(50.0, 60);
        assert!(
            history
                .ingest(
                    "h",
                    "Host",
                    &sample(95.0, 95.0, 100.0),
                    1_000,
                    &prefs,
                    false
                )
                .is_empty()
        );
        assert!(
            history
                .ingest("h", "Host", &sample(95.0, 95.0, 100.0), 2_000, &prefs, true)
                .is_empty()
        );
        let events = history.ingest("h", "Host", &sample(95.0, 95.0, 100.0), 2_061, &prefs, true);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn observer_queues_alerts_and_history_for_the_engine() {
        let (observer, webhook) = HostObserver::new();
        observer.configure(&preferences(90.0, 60), true);
        observer.observe_at("h", "Host", &sample(95.0, 10.0, 100.0), 1_000);
        assert!(webhook.try_recv().is_err());
        observer.observe_at("h", "Host", &sample(95.0, 10.0, 100.0), 1_060);
        let event = webhook.try_recv().unwrap();
        assert_eq!(event.host_id, "h");
        assert_eq!(observer.take(0).len(), 1);
        assert!(observer.take(0).is_empty());
        assert_eq!(observer.history(None)["hosts"][0]["id"], "h");
        // Without a webhook subscription the system notification queue still fills.
        observer.configure(&preferences(90.0, 60), false);
        observer.observe_at("h", "Host", &sample(40.0, 10.0, 100.0), 1_100);
        observer.observe_at("h", "Host", &sample(95.0, 10.0, 100.0), 1_200);
        observer.observe_at("h", "Host", &sample(95.0, 10.0, 100.0), 1_260);
        assert!(webhook.try_recv().is_err());
        assert_eq!(observer.take(1_260).len(), 1);
        // An alert older than the notification session is dropped rather than replayed.
        observer.observe_at("h", "Host", &sample(40.0, 10.0, 100.0), 1_300);
        observer.observe_at("h", "Host", &sample(95.0, 10.0, 100.0), 1_400);
        observer.observe_at("h", "Host", &sample(95.0, 10.0, 100.0), 1_460);
        assert!(observer.take(1_461).is_empty());
        assert!(observer.take(0).is_empty());
        observer.retain(&HashSet::new());
        assert!(
            observer.history(None)["hosts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
