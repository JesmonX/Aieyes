//! Shared pet projection using the public Codex 8 x 9 animation layout.
use crate::sessions::{Phase, Snapshot};
use serde::{Deserialize, Serialize};
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
    let priority = |phase: Phase| match phase {
        Phase::NeedsInput => 0,
        Phase::Failed => 1,
        Phase::Complete => 2,
        Phase::Working | Phase::Thinking | Phase::Tool => 3,
        Phase::Interrupted => 4,
        Phase::Unknown => 5,
    };
    let selected = snapshot
        .sessions
        .iter()
        .min_by_key(|s| (priority(s.phase), std::cmp::Reverse(s.updated_at)));
    let phase = selected.map(|s| s.phase);
    let (behavior, label, row, frames, frame_ms) = if snapshot.unavailable {
        ("unknown", "状态不全", 6, 6, 260)
    } else {
        match phase {
            Some(Phase::NeedsInput) => ("attention", "等待输入", 3, 4, 180),
            Some(Phase::Failed) => ("blocked", "执行失败", 5, 8, 180),
            Some(Phase::Complete) => ("success", "已完成", 4, 5, 180),
            Some(Phase::Thinking) => ("thinking", "思考中", 8, 6, 220),
            Some(Phase::Tool) => ("working", "执行工具", 7, 6, 150),
            Some(Phase::Working) => ("running", "进行中", 1, 8, 150),
            Some(Phase::Interrupted) => ("rest", "已中断", 5, 8, 220),
            Some(Phase::Unknown) => ("unknown", "状态待确认", 6, 6, 260),
            None => ("idle", "空闲", 0, 6, 900),
        }
    };
    let key = format!(
        "{}:{}",
        selected.map(|s| s.id.as_str()).unwrap_or(""),
        behavior
    );
    PetState {
        behavior: behavior.into(),
        label: label.into(),
        row,
        frames,
        frame_ms,
        key,
    }
}
/// Three activity cycles settle into idle. Polls must not restart this timeline.
pub fn frame(state: &PetState, elapsed_ms: u64, reduced: bool) -> u32 {
    if reduced {
        return u32::from(state.row) * 8;
    }
    let cycle = u64::from(state.frames) * state.frame_ms;
    if state.row != 0 && elapsed_ms >= cycle * 3 {
        return ((elapsed_ms - cycle * 3) / 900 % 6) as u32;
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
            })
            .collect(),
            unavailable: false,
        };
        assert_eq!(project(&snapshot).behavior, "attention");
        snapshot.sessions.pop();
        assert_eq!(project(&snapshot).behavior, "blocked");
        snapshot.sessions.pop();
        assert_eq!(project(&snapshot).behavior, "success");
        snapshot.unavailable = true;
        assert_eq!(project(&snapshot).behavior, "unknown");
    }
    #[test]
    fn frames_are_bounded_and_motion_settles() {
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
                }],
                unavailable: false,
            });
            assert_eq!(frame(&state, 0, true), u32::from(state.row) * 8);
            assert_eq!(frame(&state, 999999, true), frame(&state, 0, true));
            assert!(frame(&state, 100000, false) < 6);
            for time in 0..10000 {
                assert!(frame(&state, time, false) < 72);
            }
        }
    }
}
