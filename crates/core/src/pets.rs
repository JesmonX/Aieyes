//! Shared pet projection using the public Codex 8 x 9 animation layout.
use crate::contracts::{PET_FRAME_DURATIONS, PROVIDERS};
use crate::models::{QuotaSnapshot, Settings};
use crate::quota_ring::{self, FIVE_HOURS, Ring, WEEK};
use crate::sessions::{Phase, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
const FRAME_MS: u64 = 100;
pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 2.0;
pub const BASE_WIDTH: f64 = 144.0;
pub const ORBIT_PAD_X: f64 = 44.0;
pub const ORBIT_PAD_TOP: f64 = 44.0;
pub const ORBIT_NODE_SIZE: f64 = 32.0;
pub const ORBIT_MAX_SLOTS: usize = 7;
pub const ORBIT_OVERFLOW_PROVIDER: &str = "__overflow__";
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetOrbitQuota {
    pub account_name: String,
    pub provider: String,
    pub ring: Ring,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetOrbitAgent {
    pub provider: String,
    pub display_provider: String,
    pub phase: Phase,
    pub active_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overflow_providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<PetOrbitQuota>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetOrbitSlot {
    pub cx: f64,
    pub cy: f64,
    pub angle_deg: f64,
}

/// Transparent padding around the base 144 × height canvas when running agents orbit the pet.
/// Idle pets keep (0, 0) so existing compact bounds and tests remain unchanged.
pub fn orbit_padding(has_agents: bool) -> (f64, f64) {
    if has_agents {
        (ORBIT_PAD_X, ORBIT_PAD_TOP)
    } else {
        (0.0, 0.0)
    }
}

pub fn canvas_size(state: &PetState, has_agents: bool) -> (f64, f64) {
    let (pad_x, pad_top) = orbit_padding(has_agents);
    (BASE_WIDTH + pad_x * 2.0, height(state) + pad_top)
}

/// Positions along the left-top-right arc around the pet sprite at scale 1.0.
pub fn orbit_slots(count: usize) -> Vec<PetOrbitSlot> {
    if count == 0 {
        return Vec::new();
    }
    let angles: &[f64] = match count {
        1 => &[-90.0],
        2 => &[-132.0, -48.0],
        3 => &[-155.0, -90.0, -25.0],
        4 => &[-168.0, -116.0, -64.0, -12.0],
        5 => &[-176.0, -133.0, -90.0, -47.0, -4.0],
        6 => &[-180.0, -144.0, -108.0, -72.0, -36.0, 0.0],
        _ => &[-184.0, -153.0, -121.0, -90.0, -59.0, -27.0, 4.0],
    };
    let cx0 = ORBIT_PAD_X + 72.0;
    let cy0 = ORBIT_PAD_TOP + 78.0;
    let rx = 84.0;
    let ry = 76.0;
    angles
        .iter()
        .take(count.min(ORBIT_MAX_SLOTS))
        .map(|&angle_deg| {
            let rad = angle_deg.to_radians();
            PetOrbitSlot {
                cx: cx0 + rx * rad.cos(),
                cy: cy0 + ry * rad.sin(),
                angle_deg,
            }
        })
        .collect()
}

pub fn agent_provider(provider: &str) -> &str {
    crate::contracts::provider(provider)
        .map(|entry| entry.agent_id)
        .unwrap_or(provider)
}

fn provider_order(agent_id: &str) -> usize {
    PROVIDERS
        .iter()
        .position(|entry| entry.agent_id == agent_id)
        .unwrap_or(usize::MAX)
}

fn arc_summary(arc: &quota_ring::Arc, now: i64, include_window: bool) -> String {
    let prefix = if include_window {
        match arc.window_minutes {
            Some(FIVE_HOURS) => "5h ",
            Some(WEEK) => "7d ",
            _ => "",
        }
    } else {
        ""
    };
    let remaining = quota_ring::remaining_text(arc);
    let reset = quota_ring::reset_text(arc, now);
    if reset == "—" || reset == "5h" || reset == "7d" {
        format!("{prefix}{remaining}")
    } else {
        format!("{prefix}{remaining} ({reset})")
    }
}

pub fn format_quota_summary(ring: &Ring, now: i64) -> String {
    match ring {
        Ring::Duo { top, bottom } => format!(
            "{} · {}",
            arc_summary(top, now, true),
            arc_summary(bottom, now, true)
        ),
        Ring::Single { top } => arc_summary(top, now, false),
        Ring::Coin { amount, .. } => amount.clone(),
        Ring::Empty => String::new(),
    }
}

fn match_quota(
    agent_id: &str,
    source_ids: &[&str],
    settings: &Settings,
    quotas: &[QuotaSnapshot],
    now: i64,
) -> Option<PetOrbitQuota> {
    let mut candidate_accounts = Vec::<&str>::new();
    for source_id in source_ids {
        if let Some(source) = settings.sources.iter().find(|s| s.id == *source_id)
            && !source.account_id.is_empty()
            && !candidate_accounts.contains(&source.account_id.as_str())
        {
            candidate_accounts.push(source.account_id.as_str());
        }
    }
    let build = |quota: &QuotaSnapshot| -> Option<PetOrbitQuota> {
        let ring = quota_ring::ring(quota);
        if ring == Ring::Empty {
            return None;
        }
        Some(PetOrbitQuota {
            account_name: quota.name.clone(),
            provider: quota.provider.clone(),
            summary: format_quota_summary(&ring, now),
            ring,
        })
    };
    for account_id in candidate_accounts {
        if let Some(quota) = quotas
            .iter()
            .find(|q| agent_provider(&q.provider) == agent_id && q.account_id == account_id)
            && let Some(item) = build(quota)
        {
            return Some(item);
        }
    }
    quotas
        .iter()
        .filter(|q| {
            agent_provider(&q.provider) == agent_id
                && !(settings.hide_free_quota_cards && q.free_plan)
        })
        .find_map(build)
}

/// Group running sessions (`phase.active()`) by normalized agent, attach matched quota rings,
/// keep deterministic order, and fold beyond [`ORBIT_MAX_SLOTS`] into an overflow badge.
pub fn project_orbit(
    snapshot: &Snapshot,
    settings: &Settings,
    quotas: &[QuotaSnapshot],
    now: i64,
) -> Vec<PetOrbitAgent> {
    struct Bucket<'a> {
        agent_id: String,
        display_provider: String,
        all_same_raw: bool,
        phase: Phase,
        active_count: usize,
        source_ids: Vec<&'a str>,
    }
    let mut grouped = BTreeMap::<String, Bucket<'_>>::new();
    for session in snapshot.sessions.iter().filter(|s| s.phase.active()) {
        let agent_id = agent_provider(&session.provider).to_owned();
        let entry = grouped.entry(agent_id.clone()).or_insert_with(|| Bucket {
            agent_id,
            display_provider: session.provider.clone(),
            all_same_raw: true,
            phase: session.phase,
            active_count: 0,
            source_ids: Vec::new(),
        });
        entry.active_count += 1;
        if session.phase.priority() < entry.phase.priority() {
            entry.phase = session.phase;
        }
        if entry.display_provider != session.provider {
            entry.all_same_raw = false;
        }
        if !session.source_id.is_empty() && !entry.source_ids.contains(&session.source_id.as_str())
        {
            entry.source_ids.push(session.source_id.as_str());
        }
    }
    let mut items: Vec<PetOrbitAgent> = grouped
        .into_values()
        .map(|bucket| {
            let display_provider = if bucket.all_same_raw {
                bucket.display_provider
            } else {
                bucket.agent_id.clone()
            };
            let quota = match_quota(&bucket.agent_id, &bucket.source_ids, settings, quotas, now);
            PetOrbitAgent {
                provider: bucket.agent_id,
                display_provider,
                phase: bucket.phase,
                active_count: bucket.active_count,
                overflow_providers: Vec::new(),
                quota,
            }
        })
        .collect();
    items.sort_by(|a, b| {
        provider_order(&a.provider)
            .cmp(&provider_order(&b.provider))
            .then_with(|| a.provider.cmp(&b.provider))
    });
    if items.len() > ORBIT_MAX_SLOTS {
        let rest = items.split_off(ORBIT_MAX_SLOTS - 1);
        let mut phase = rest[0].phase;
        let mut active_count = 0;
        let mut overflow_providers = Vec::with_capacity(rest.len());
        for item in rest {
            if item.phase.priority() < phase.priority() {
                phase = item.phase;
            }
            active_count += item.active_count;
            overflow_providers.push(item.provider);
        }
        items.push(PetOrbitAgent {
            provider: ORBIT_OVERFLOW_PROVIDER.to_owned(),
            display_provider: ORBIT_OVERFLOW_PROVIDER.to_owned(),
            phase,
            active_count,
            overflow_providers,
            quota: None,
        });
    }
    items
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

    #[test]
    fn orbit_slots_stay_within_expanded_canvas_bounds() {
        let idle = project(&Snapshot::default());
        assert_eq!(canvas_size(&idle, false), (144.0, 168.0));
        assert_eq!(canvas_size(&idle, true), (232.0, 212.0));
        assert!(orbit_slots(0).is_empty());
        let (width, _) = canvas_size(&idle, true);
        let half = ORBIT_NODE_SIZE / 2.0;
        let halo_margin = 6.0;
        for count in 1..=ORBIT_MAX_SLOTS {
            let slots = orbit_slots(count);
            assert_eq!(slots.len(), count);
            for slot in slots {
                assert!(
                    slot.cx - half >= halo_margin && slot.cx + half <= width - halo_margin,
                    "slot x out of safe horizontal bounds for count={count}: {:?}",
                    slot
                );
                assert!(
                    slot.cy - half >= halo_margin,
                    "slot y out of safe top bounds for count={count}: {:?}",
                    slot
                );
            }
        }
    }

    #[test]
    fn orbit_groups_active_agents_matches_source_quota_and_folds_overflow() {
        use crate::models::{QuotaWindow, Source};
        let now = 1_000_000;
        let settings = Settings {
            sources: vec![Source {
                id: "src-codex-b".into(),
                provider: "codex".into(),
                account_id: "acct-b".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let quotas = vec![
            QuotaSnapshot {
                provider: "codex".into(),
                account_id: "acct-a".into(),
                name: "Codex A".into(),
                windows: vec![QuotaWindow {
                    name: "5h".into(),
                    used_percent: 10.0,
                    window_minutes: Some(FIVE_HOURS),
                    resets_at: Some(now + 3600),
                    ..Default::default()
                }],
                ..Default::default()
            },
            QuotaSnapshot {
                provider: "codex".into(),
                account_id: "acct-b".into(),
                name: "Codex B".into(),
                windows: vec![
                    QuotaWindow {
                        name: "5h".into(),
                        used_percent: 35.0,
                        window_minutes: Some(FIVE_HOURS),
                        resets_at: Some(now + 7200),
                        ..Default::default()
                    },
                    QuotaWindow {
                        name: "7d".into(),
                        used_percent: 42.0,
                        window_minutes: Some(WEEK),
                        resets_at: None,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ];
        let snapshot = Snapshot {
            sessions: vec![
                LiveSession {
                    id: "1".into(),
                    provider: "claude".into(),
                    phase: Phase::Working,
                    updated_at: 100,
                    ..Default::default()
                },
                LiveSession {
                    id: "2".into(),
                    provider: "codex".into(),
                    source_id: "src-codex-b".into(),
                    phase: Phase::Thinking,
                    updated_at: 101,
                    ..Default::default()
                },
                LiveSession {
                    id: "3".into(),
                    provider: "chatgpt-work".into(),
                    phase: Phase::NeedsInput,
                    updated_at: 102,
                    ..Default::default()
                },
                LiveSession {
                    id: "4".into(),
                    provider: "cursor".into(),
                    phase: Phase::Complete,
                    updated_at: 103,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let orbit = project_orbit(&snapshot, &settings, &quotas, now);
        assert_eq!(
            orbit.len(),
            2,
            "complete sessions excluded; aliases grouped"
        );
        assert_eq!(orbit[0].provider, "codex");
        assert_eq!(orbit[0].display_provider, "codex");
        assert_eq!(orbit[0].active_count, 2);
        assert_eq!(orbit[0].phase, Phase::NeedsInput);
        let quota = orbit[0].quota.as_ref().expect("matched source account");
        assert_eq!(quota.account_name, "Codex B");
        assert_eq!(quota.summary, "5h 65% (2h) · 7d 58%");
        assert_eq!(orbit[1].provider, "claude");
        assert!(orbit[1].quota.is_none());

        // Overflow folding past 7 running agents.
        let many_providers = [
            "codex",
            "claude",
            "antigravity",
            "cursor",
            "opencode",
            "kimi",
            "qoder",
            "grok",
            "pi",
        ];
        let many = Snapshot {
            sessions: many_providers
                .iter()
                .enumerate()
                .map(|(i, p)| LiveSession {
                    id: format!("s-{i}"),
                    provider: (*p).into(),
                    phase: if *p == "pi" {
                        Phase::NeedsInput
                    } else {
                        Phase::Working
                    },
                    updated_at: 100 + i as u64,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let folded = project_orbit(&many, &Settings::default(), &[], now);
        assert_eq!(folded.len(), ORBIT_MAX_SLOTS);
        let tail = folded.last().unwrap();
        assert_eq!(tail.provider, ORBIT_OVERFLOW_PROVIDER);
        assert_eq!(tail.overflow_providers.len(), 3);
        assert_eq!(tail.active_count, 3);
        assert_eq!(tail.phase, Phase::NeedsInput);
    }
}
