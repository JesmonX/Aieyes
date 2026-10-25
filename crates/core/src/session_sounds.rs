//! Host-level alerts: polling and additional surfaces must never replay old sounds.
use crate::sessions::{Phase, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub needs_input: bool,
    pub complete: bool,
    pub needs_input_tone: String,
    pub complete_tone: String,
    pub volume: f64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            needs_input: true,
            complete: true,
            needs_input_tone: "default".into(),
            complete_tone: "default".into(),
            volume: 1.0,
        }
    }
}
impl Preferences {
    pub fn normalize(&mut self) {
        self.volume = normalized_volume(self.volume);
        for tone in [&mut self.needs_input_tone, &mut self.complete_tone] {
            if sound_file("complete", tone).is_none() {
                *tone = "default".into();
            }
        }
    }
    pub fn tone(&self, kind: &str) -> &str {
        let tone = if kind == "needsInput" {
            &self.needs_input_tone
        } else {
            &self.complete_tone
        };
        if sound_file(kind, tone).is_some() {
            tone
        } else {
            "default"
        }
    }
}
pub fn normalized_volume(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}
pub fn sound_file(kind: &str, tone: &str) -> Option<&'static str> {
    let default = match kind {
        "needsInput" => "needs-input",
        "complete" => "complete",
        _ => return None,
    };
    crate::contracts::SESSION_SOUND_TONES
        .iter()
        .find(|row| row.0 == tone)
        .map(|row| if tone == "default" { default } else { row.2 })
}
/// Apply per-app gain to bundled PCM samples, leaving the system output volume alone.
pub fn scale_pcm16(bytes: &[u8], volume: f64) -> Result<Vec<u8>, &'static str> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("无效提示音资源");
    }
    let mut offset = 12usize;
    let mut pcm16 = false;
    let mut data = None;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or("无效提示音资源")?;
        match &bytes[offset..offset + 4] {
            b"fmt " if size >= 16 => {
                pcm16 = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap()) == 1
                    && u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()) == 16;
            }
            b"data" => data = Some(start..end),
            _ => {}
        }
        offset = end + size % 2;
    }
    let data = data
        .filter(|range| pcm16 && range.len() % 2 == 0)
        .ok_or("无效提示音资源")?;
    let mut result = bytes.to_vec();
    let gain = normalized_volume(volume);
    for sample in result[data].as_chunks_mut::<2>().0 {
        let scaled = (f64::from(i16::from_le_bytes(*sample)) * gain).round() as i16;
        sample.copy_from_slice(&scaled.to_le_bytes());
    }
    Ok(result)
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
    fn legacy_preferences_and_invalid_choices_keep_safe_defaults() {
        let old: Preferences =
            serde_json::from_str(r#"{"needsInput":false,"complete":true}"#).unwrap();
        assert!(!old.needs_input && old.complete);
        assert_eq!(old.volume, 1.0);
        assert_eq!(
            sound_file("needsInput", old.tone("needsInput")),
            Some("needs-input")
        );
        let mut changed: Preferences = serde_json::from_str(
            r#"{"needsInputTone":"bell","completeTone":"unknown","volume":2}"#,
        )
        .unwrap();
        changed.normalize();
        assert_eq!(changed.tone("needsInput"), "bell");
        assert_eq!(changed.tone("complete"), "default");
        assert_eq!(changed.volume, 1.0);
        changed.volume = -0.5;
        changed.normalize();
        assert_eq!(changed.volume, 0.0);
        assert_eq!(normalized_volume(f64::NAN), 1.0);
    }
    #[test]
    fn per_app_volume_scales_samples_and_preserves_wave_metadata() {
        let original = include_bytes!("../../../apps/desktop/web/sounds/complete.wav");
        assert_eq!(scale_pcm16(original, 1.0).unwrap(), original);
        let half = scale_pcm16(original, 0.5).unwrap();
        assert_eq!(&half[..44], &original[..44]);
        for (before, after) in original[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .zip(half[44..].as_chunks::<2>().0)
        {
            let before = i16::from_le_bytes(*before);
            let after = i16::from_le_bytes(*after);
            assert!((f64::from(after) - f64::from(before) * 0.5).abs() <= 0.5);
        }
        assert!(
            scale_pcm16(original, 0.0).unwrap()[44..]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(scale_pcm16(&original[..30], 0.5).is_err());
        let mut invalid = original.to_vec();
        invalid[34..36].copy_from_slice(&8u16.to_le_bytes());
        assert!(scale_pcm16(&invalid, 0.5).is_err());
    }
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
                        complete: false,
                        ..Default::default()
                    }
                )
                .is_empty()
        );
        assert!(tracker.update(&snapshot, 109, &prefs).is_empty());
    }
}
