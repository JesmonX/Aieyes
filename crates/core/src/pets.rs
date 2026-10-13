//! Shared pet projection using the public Codex 8 x 9 animation layout.
use crate::sessions::{Phase, Snapshot};
use serde::{Deserialize, Serialize};
const FRAME_MS: u64 = 180;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetState {
    pub behavior: String,
    pub label: String,
    pub row: u8,
    pub frames: u8,
    pub frame_ms: u64,
    pub key: String,
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
            Some(Phase::Interrupted) => ("rest", "已中断", 5, 8),
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
    PetState {
        behavior: behavior.into(),
        label: label.into(),
        row,
        frames,
        frame_ms: FRAME_MS,
        key,
    }
}
/// Loop the selected behavior until the session state changes, keeping its label accurate.
pub fn frame(state: &PetState, elapsed_ms: u64, reduced: bool) -> u32 {
    if reduced {
        return u32::from(state.row) * 8;
    }
    u32::from(state.row) * 8 + (elapsed_ms / state.frame_ms % u64::from(state.frames)) as u32
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::LiveSession;
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
    fn frames_use_one_cadence_and_preserve_the_selected_state() {
        let idle = project(&Snapshot::default());
        let idle_cadence: Vec<_> = (0..=6)
            .map(|step| frame(&idle, step * 180 + 1, false))
            .collect();
        assert_eq!(idle_cadence, [0, 1, 2, 3, 4, 5, 0]);
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
            for step in 0..u64::from(state.frames) * 8 {
                assert_eq!(
                    frame(&state, step * 180 + 1, false),
                    u32::from(state.row) * 8 + (step % u64::from(state.frames)) as u32
                );
            }
            assert_eq!(frame(&state, 100000, false) / 8, u32::from(state.row));
            for time in 0..10000 {
                assert!(frame(&state, time, false) < 72);
            }
        }
    }
}
