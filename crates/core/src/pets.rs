//! Shared pet projection using the public Codex 8 x 9 animation layout.
use crate::contracts::PET_FRAME_DURATIONS;
use crate::sessions::{Phase, Snapshot};
use serde::{Deserialize, Serialize};
const FRAME_MS: u64 = 100;
pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 2.0;
pub fn normalized_scale(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetState {
    pub behavior: String,
    pub label: String,
    pub row: u8,
    pub frames: u8,
    pub frame_ms: u64,
    #[serde(default)]
    pub frame_durations_ms: Vec<u64>,
    pub key: String,
    #[serde(default)]
    pub active_count: usize,
    #[serde(default)]
    pub phase_counts: std::collections::BTreeMap<String, usize>,
}
pub fn project(snapshot: &Snapshot) -> PetState {
    let selected = snapshot.selected();
    let phase = selected.map(|s| s.phase);
    let (behavior, label, row, frames) = if snapshot.unavailable && selected.is_none() {
        ("unknown", "状态不全", 6, 6)
    } else {
        match phase {
            Some(Phase::NeedsInput) => ("attention", "等待输入", 3, 4),
            Some(Phase::Failed) => ("blocked", "执行失败", 5, 8),
            Some(Phase::Complete) => ("success", "已完成", 4, 5),
            Some(Phase::Thinking) => ("thinking", "思考中", 8, 6),
            Some(Phase::Tool) => ("working", "执行工具", 7, 6),
            Some(Phase::Working) => ("running", "进行中", 1, 8),
            Some(Phase::Interrupted) => ("rest", "已中断", 0, 6),
            Some(Phase::Unknown) => ("unknown", "状态待确认", 6, 6),
            None => ("idle", "空闲", 0, 6),
        }
    };
    let key = format!(
        "{}:{}:{}",
        selected.map(|s| s.id.as_str()).unwrap_or(""),
        selected.map(|s| s.turn_id.as_str()).unwrap_or(""),
        behavior
    );
    let mut label = if snapshot.sessions.len() > 1 {
        snapshot
            .count_labels()
            .chunks(2)
            .map(|line| line.join(" · "))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        label.into()
    };
    if snapshot.unavailable && selected.is_some() {
        label.push_str("\n部分状态不可用");
    }
    PetState {
        behavior: behavior.into(),
        label,
        row,
        frames,
        frame_ms: FRAME_MS,
        frame_durations_ms: PET_FRAME_DURATIONS[row as usize].to_vec(),
        key,
        active_count: snapshot.active_count(),
        phase_counts: snapshot.phase_counts(),
    }
}
pub fn same_animation(a: &PetState, b: &PetState) -> bool {
    a.row == b.row && a.frames == b.frames && effective_durations(a) == effective_durations(b)
}
fn effective_durations(state: &PetState) -> Vec<u64> {
    let values = if state.frame_durations_ms.is_empty() {
        PET_FRAME_DURATIONS
            .get(state.row as usize)
            .copied()
            .unwrap_or(&[])
    } else {
        &state.frame_durations_ms
    };
    if values.len() == usize::from(state.frames)
        && values.iter().all(|v| *v > 0)
        && values
            .iter()
            .try_fold(0u64, |a, b| a.checked_add(*b))
            .is_some()
        && !values.is_empty()
    {
        values.to_vec()
    } else {
        vec![state.frame_ms.max(1); usize::from(state.frames.max(1))]
    }
}
pub fn height(state: &PetState) -> f64 {
    168.0 + state.label.lines().count().saturating_sub(1) as f64 * 18.0
}
/// Loop the selected behavior until the session state changes, keeping its label accurate.
pub fn frame(state: &PetState, elapsed_ms: u64, reduced: bool) -> u32 {
    if reduced {
        return u32::from(state.row) * 8;
    }
    u32::from(state.row) * 8 + frame_timing(state, elapsed_ms).0
}
/// Time to the next actual atlas frame; static/reduced states need no timer.
pub fn next_frame_delay(state: &PetState, elapsed_ms: u64, reduced: bool) -> Option<u64> {
    (!reduced && state.frames > 1).then(|| frame_timing(state, elapsed_ms).1)
}
fn frame_timing(state: &PetState, elapsed_ms: u64) -> (u32, u64) {
    let durations = if state.frame_durations_ms.is_empty() {
        PET_FRAME_DURATIONS
            .get(state.row as usize)
            .copied()
            .unwrap_or(&[])
    } else {
        state.frame_durations_ms.as_slice()
    };
    if !durations.is_empty()
        && durations.len() == usize::from(state.frames)
        && durations.iter().all(|duration| *duration > 0)
        && let Some(cycle) = durations
            .iter()
            .try_fold(0u64, |sum, d| sum.checked_add(*d))
    {
        let mut remainder = elapsed_ms % cycle;
        for (index, duration) in durations.iter().enumerate() {
            if remainder < *duration {
                return (index as u32, duration - remainder);
            }
            remainder -= duration;
        }
    }
    let duration = state.frame_ms.max(1);
    (
        (elapsed_ms / duration % u64::from(state.frames.max(1))) as u32,
        duration - elapsed_ms % duration,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::LiveSession;
    #[test]
    fn animation_identity_ignores_sessions_and_uses_effective_timing() {
        let a = project(&Snapshot::default());
        let mut b = a.clone();
        b.key = "another-session:another-turn".into();
        b.label = "Changed".into();
        assert!(same_animation(&a, &b));
        b.frame_durations_ms.clear();
        assert!(same_animation(&a, &b));
        b.frame_durations_ms = vec![100; 6];
        assert!(!same_animation(&a, &b));
        b = a.clone();
        b.row = 1;
        assert!(!same_animation(&a, &b));
    }
    #[test]
    fn timers_land_on_frame_boundaries_and_stop_for_static_states() {
        let mut state = project(&Snapshot::default());
        for durations in [
            vec![280, 110, 110, 140, 140, 320],
            vec![0; 6],
            vec![u64::MAX; 6],
            vec![7, 11],
        ] {
            state.frame_durations_ms = durations;
            for elapsed in 0..2200 {
                let delay = next_frame_delay(&state, elapsed, false).unwrap();
                assert!(delay > 0);
                assert_eq!(
                    frame(&state, elapsed, false),
                    frame(&state, elapsed + delay - 1, false)
                );
                assert_ne!(
                    frame(&state, elapsed, false),
                    frame(&state, elapsed + delay, false)
                );
            }
        }
        assert_eq!(next_frame_delay(&state, 20, true), None);
        state.frames = 1;
        assert_eq!(next_frame_delay(&state, 20, false), None);
    }
    #[test]
    fn attention_priority_and_unknown_are_not_working() {
        let mut snapshot = Snapshot {
            sessions: vec![
                Phase::Working,
                Phase::Complete,
                Phase::Failed,
                Phase::NeedsInput,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, phase)| LiveSession {
                id: i.to_string(),
                source: "fixture".into(),
                phase,
                updated_at: 100,
                ..LiveSession::default()
            })
            .collect(),
            unavailable: false,
            ..Snapshot::default()
        };
        assert_eq!(project(&snapshot).behavior, "attention");
        snapshot.sessions.pop();
        assert_eq!(project(&snapshot).behavior, "blocked");
        snapshot.sessions.pop();
        assert_eq!(project(&snapshot).behavior, "running");
        snapshot.unavailable = true;
        assert_eq!(project(&snapshot).behavior, "running");
        snapshot.sessions.clear();
        assert_eq!(project(&snapshot).behavior, "unknown");
    }
    #[test]
    fn frames_follow_shared_durations_and_preserve_the_selected_state() {
        let idle = project(&Snapshot::default());
        let idle_cadence: Vec<_> = [0, 279, 280, 389, 390, 500, 640, 780, 1099, 1100]
            .into_iter()
            .map(|ms| frame(&idle, ms, false))
            .collect();
        assert_eq!(idle_cadence, [0, 0, 1, 1, 2, 3, 4, 5, 5, 0]);
        for phase in [
            Phase::Working,
            Phase::Thinking,
            Phase::Tool,
            Phase::Complete,
            Phase::Interrupted,
            Phase::Unknown,
            Phase::NeedsInput,
            Phase::Failed,
        ] {
            let state = project(&Snapshot {
                sessions: vec![LiveSession {
                    id: "s".into(),
                    source: "local".into(),
                    phase,
                    updated_at: 1,
                    ..LiveSession::default()
                }],
                unavailable: false,
                ..Snapshot::default()
            });
            assert_eq!(frame(&state, 0, true), u32::from(state.row) * 8);
            assert_eq!(frame(&state, 999999, true), frame(&state, 0, true));
            assert_eq!(state.frame_ms, idle.frame_ms);
            let cycle: u64 = state.frame_durations_ms.iter().sum();
            for repeat in 0..8 {
                let mut start = cycle * repeat;
                for (index, duration) in state.frame_durations_ms.iter().enumerate() {
                    let expected = u32::from(state.row) * 8 + index as u32;
                    assert_eq!(frame(&state, start, false), expected);
                    assert_eq!(frame(&state, start + duration - 1, false), expected);
                    start += duration;
                }
            }
            assert_eq!(frame(&state, 100000, false) / 8, u32::from(state.row));
            for time in 0..10000 {
                assert!(frame(&state, time, false) < 72);
            }
        }
    }
    #[test]
    fn counts_cover_every_phase_without_resetting_the_chosen_animation() {
        let mut snapshot = Snapshot {
            sessions: vec![LiveSession {
                id: "waiting".into(),
                turn_id: "turn".into(),
                phase: Phase::NeedsInput,
                updated_at: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        let first = project(&snapshot);
        for (index, phase) in [
            Phase::Working,
            Phase::Working,
            Phase::Thinking,
            Phase::Tool,
            Phase::Failed,
            Phase::Complete,
            Phase::Interrupted,
            Phase::Unknown,
        ]
        .into_iter()
        .enumerate()
        {
            snapshot.sessions.push(LiveSession {
                id: index.to_string(),
                phase,
                updated_at: 100,
                ..Default::default()
            });
        }
        let many = project(&snapshot);
        assert_eq!(many.key, first.key);
        assert_eq!(many.active_count, 5);
        assert_eq!(many.phase_counts["working"], 2);
        assert_eq!(many.phase_counts["failed"], 1);
        assert_eq!(
            many.label,
            "等待1 · 失败1\n工作2 · 思考1\n工具1 · 完成1\n中断1 · 待定1"
        );
        assert_eq!(height(&many), 222.0);
        snapshot.unavailable = true;
        assert_eq!(project(&snapshot).key, first.key);
        assert!(project(&snapshot).label.ends_with("部分状态不可用"));
        snapshot.sessions.retain(|s| s.phase == Phase::Interrupted);
        assert_eq!(project(&snapshot).row, 0);
        assert_eq!(
            project(&snapshot).frame_durations_ms,
            PET_FRAME_DURATIONS[0]
        );
        assert_eq!(project(&snapshot).active_count, 0);
        assert_eq!(normalized_scale(f64::NAN), 1.0);
        assert_eq!(normalized_scale(0.0), 0.5);
        assert_eq!(normalized_scale(3.0), 2.0);
    }

    #[test]
    fn legacy_and_invalid_duration_arrays_use_safe_fallbacks() {
        let mut state = project(&Snapshot::default());
        state.frame_durations_ms.clear();
        assert_eq!(frame(&state, 279, false), 0);
        assert_eq!(frame(&state, 280, false), 1);
        state.frame_durations_ms = vec![0; 6];
        assert_eq!(frame(&state, 100, false), 1);
        state.frame_durations_ms = vec![u64::MAX; 6];
        assert_eq!(frame(&state, 100, false), 1);
        state.row = 9;
        state.frames = 0;
        state.frame_ms = 0;
        state.frame_durations_ms.clear();
        assert_eq!(frame(&state, 100, false), 72);
    }
}
