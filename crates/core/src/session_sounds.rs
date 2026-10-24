//! Host-level alerts: polling and additional surfaces must never replay old sounds.
use crate::sessions::{Phase, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub needs_input: bool,
    pub complete: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            needs_input: true,
            complete: true,
        }
    }
}
type SessionKey = (String, String, String, String);
type LastState = (String, Phase, u64);
#[derive(Default)]
pub struct Tracker {
    initialized: bool,
    started_at: u64,
    seen: HashMap<SessionKey, LastState>,
    unhealthy: HashSet<(String, String)>,
}
impl Tracker {
    pub fn update(
        &mut self,
        snapshot: &Snapshot,
        now: u64,
        prefs: &Preferences,
    ) -> Vec<&'static str> {
        if !self.initialized {
            self.started_at = now;
        }
        let unhealthy: HashSet<_> = snapshot
            .sources
            .iter()
            .filter(|s| s.state != "healthy")
            .map(|s| (s.machine_id.clone(), s.source_id.clone()))
            .collect();
        let mut alerts = Vec::new();
        for s in &snapshot.sessions {
            let key = (
                s.provider.clone(),
                s.machine_id.clone(),
                s.source_id.clone(),
                s.id.clone(),
            );
            let source = (s.machine_id.clone(), s.source_id.clone());
            let prior = self.seen.get(&key);
            let changed = prior.is_none_or(|(turn, phase, at)| {
                (turn != &s.turn_id || phase != &s.phase) && s.updated_at >= *at
            });
            let fresh =
                s.updated_at >= self.started_at && s.updated_at <= now && now - s.updated_at <= 10;
            if self.initialized
                && fresh
                && changed
                && !unhealthy.contains(&source)
                && !self.unhealthy.contains(&source)
            {
                let kind = match s.phase {
                    Phase::NeedsInput if prefs.needs_input => Some("needsInput"),
                    Phase::Complete if prefs.complete => Some("complete"),
                    _ => None,
                };
                if let Some(kind) = kind
                    && !alerts.contains(&kind)
                {
                    alerts.push(kind);
                }
            }
            if prior.is_none_or(|(_, _, at)| s.updated_at >= *at) {
                self.seen
                    .insert(key, (s.turn_id.clone(), s.phase, s.updated_at));
            }
        }
        self.seen
            .retain(|_, (_, _, at)| now.saturating_sub(*at) <= 86400);
        self.unhealthy = unhealthy;
        self.initialized = true;
        alerts
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::LiveSession;
    #[test]
    fn alerts_are_fresh_per_turn_and_survive_missing_snapshots_without_replay() {
        let mut tracker = Tracker::default();
        let mut snapshot = Snapshot {
            sessions: vec![LiveSession {
                id: "s".into(),
                turn_id: "t".into(),
                phase: Phase::Complete,
                updated_at: 100,
                ..Default::default()
            }],
            ..Default::default()
        };
        let prefs = Preferences::default();
        assert!(tracker.update(&snapshot, 100, &prefs).is_empty());
        snapshot.sessions[0].phase = Phase::Working;
        snapshot.sessions[0].updated_at = 101;
        assert!(tracker.update(&snapshot, 101, &prefs).is_empty());
        snapshot.sessions[0].phase = Phase::NeedsInput;
        snapshot.sessions[0].updated_at = 102;
        assert_eq!(tracker.update(&snapshot, 102, &prefs), ["needsInput"]);
        assert!(tracker.update(&snapshot, 103, &prefs).is_empty());
        assert!(tracker.update(&Snapshot::default(), 104, &prefs).is_empty());
        assert!(tracker.update(&snapshot, 105, &prefs).is_empty());
        snapshot.sessions[0].phase = Phase::Complete;
        snapshot.sessions[0].updated_at = 106;
        assert_eq!(tracker.update(&snapshot, 106, &prefs), ["complete"]);
        snapshot.sessions[0].turn_id = "next".into();
        snapshot.sessions[0].updated_at = 107;
        assert_eq!(tracker.update(&snapshot, 107, &prefs), ["complete"]);
        snapshot.sessions[0].phase = Phase::NeedsInput;
        snapshot.sessions[0].updated_at = 108;
        assert!(
            tracker
                .update(
                    &snapshot,
                    108,
                    &Preferences {
                        needs_input: false,
                        complete: false
                    }
                )
                .is_empty()
        );
        assert!(tracker.update(&snapshot, 109, &prefs).is_empty());
    }
}
