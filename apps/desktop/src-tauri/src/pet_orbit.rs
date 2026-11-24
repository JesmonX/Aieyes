//! Shared orbit hit-testing, breathing pulse curve, and color rules for Windows and Linux native pets.
#![allow(dead_code)]

use aieyes_core::pets::{ORBIT_OVERFLOW_PROVIDER, PetOrbitAgent, orbit_slots};
use aieyes_core::sessions::Phase;

pub const HIT_RADIUS: f64 = 16.5;
pub const PULSE_FRAME_MS: u64 = 66;

pub fn parse_agents(info: &serde_json::Value) -> Vec<PetOrbitAgent> {
    serde_json::from_value(info["petOrbit"].clone()).unwrap_or_default()
}

/// Returns the index of the orbit node hit at `(local_x, local_y)` in logical coordinates
/// divided by `scale` (or pass raw pixels with `scale = dpi_scale * pet_scale`).
pub fn hit_test(x: f64, y: f64, scale: f64, agents: &[PetOrbitAgent]) -> Option<usize> {
    if agents.is_empty() || scale <= 0.0 {
        return None;
    }
    let slots = orbit_slots(agents.len());
    let radius = HIT_RADIUS * scale;
    let r2 = radius * radius;
    for (index, slot) in slots.iter().enumerate() {
        let dx = x - slot.cx * scale;
        let dy = y - slot.cy * scale;
        if dx * dx + dy * dy <= r2 {
            return Some(index);
        }
    }
    None
}

/// Returns `(halo_alpha, halo_scale)` for the gentle breathing glow.
pub fn pulse_metrics(elapsed_ms: u64, index: usize, phase: Phase, reduced: bool) -> (f64, f64) {
    if reduced {
        return (0.30, 1.06);
    }
    let period = if phase == Phase::NeedsInput {
        1500.0
    } else {
        2200.0
    };
    let shifted = (elapsed_ms as f64 + index as f64 * 350.0) % period;
    // Smooth sine wave 0..1..0
    let wave = 0.5 - 0.5 * (shifted / period * std::f64::consts::TAU).cos();
    let alpha = 0.16 + 0.38 * wave;
    let scale = 0.98 + 0.18 * wave;
    (alpha, scale)
}

/// Matches macOS `Palette.knownCategory` / `DesignTokens` category colors.
pub fn provider_rgb(provider: &str, dark: bool) -> [u8; 3] {
    let hex: u32 = match (provider, dark) {
        ("codex" | "chatgpt-work", false) => 0x256b91,
        ("codex" | "chatgpt-work", true) => 0x77c4ea,
        ("claude", false) => 0xa35631,
        ("claude", true) => 0xf2b18e,
        ("antigravity", false) => 0x7953ba,
        ("antigravity", true) => 0xcaadfa,
        ("deepseek" | "deepseek-harness", false) => 0x385bc5,
        ("deepseek" | "deepseek-harness", true) => 0xa0b7ff,
        (_, false) => 0x6b6b73,
        (_, true) => 0xa4a4ae,
    };
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

pub fn quota_arc_rgb(provider: &str, remaining: f64, dark: bool) -> [u8; 3] {
    match aieyes_core::quota_ring::level(remaining) {
        "danger" => {
            if dark {
                [0xff, 0x92, 0x9c]
            } else {
                [0xb8, 0x32, 0x45]
            }
        }
        "warn" => {
            if dark {
                [0xe0, 0xb0, 0x62]
            } else {
                [0x89, 0x57, 0x12]
            }
        }
        _ => provider_rgb(provider, dark),
    }
}

pub fn pulse_rgb(agent: &PetOrbitAgent, dark: bool) -> [u8; 3] {
    if agent.phase == Phase::NeedsInput {
        return if dark {
            [0xe0, 0xb0, 0x62]
        } else {
            [0x89, 0x57, 0x12]
        };
    }
    if agent.provider == ORBIT_OVERFLOW_PROVIDER {
        return if dark {
            [0xa3, 0xb3, 0xff]
        } else {
            [0x55, 0x66, 0xd9]
        };
    }
    provider_rgb(&agent.display_provider, dark)
}

pub fn provider_display_name(provider: &str) -> String {
    if provider == ORBIT_OVERFLOW_PROVIDER {
        return crate::localization::label("其他运行中 Agent").to_owned();
    }
    aieyes_core::contracts::provider(provider)
        .map(|entry| crate::localization::text(entry.name))
        .unwrap_or_else(|| provider.to_owned())
}

pub fn node_tooltip(agent: &PetOrbitAgent) -> String {
    let mut parts = Vec::new();
    if agent.provider == ORBIT_OVERFLOW_PROVIDER {
        parts.push(
            crate::localization::label("还有 {0} 个运行中 Agent")
                .replace("{0}", &agent.overflow_providers.len().to_string()),
        );
    } else {
        parts.push(provider_display_name(&agent.display_provider));
    }
    parts.push(
        crate::localization::label("{0} 个进行中").replace("{0}", &agent.active_count.to_string()),
    );
    if let Some(quota) = &agent.quota
        && !quota.summary.is_empty()
    {
        parts.push(quota.summary.clone());
    }
    parts.push(crate::localization::label("单击查看会话").to_owned());
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_matches_slot_center_and_ignores_transparent_margins() {
        let agents = vec![
            PetOrbitAgent {
                provider: "codex".into(),
                display_provider: "codex".into(),
                phase: Phase::Working,
                active_count: 1,
                overflow_providers: vec![],
                quota: None,
            },
            PetOrbitAgent {
                provider: "claude".into(),
                display_provider: "claude".into(),
                phase: Phase::NeedsInput,
                active_count: 2,
                overflow_providers: vec![],
                quota: None,
            },
        ];
        let slots = orbit_slots(2);
        assert_eq!(hit_test(slots[0].cx, slots[0].cy, 1.0, &agents), Some(0));
        assert_eq!(
            hit_test(slots[1].cx * 1.5, slots[1].cy * 1.5, 1.5, &agents),
            Some(1)
        );
        assert_eq!(hit_test(2.0, 2.0, 1.0, &agents), None);
        let (a1, s1) = pulse_metrics(0, 0, Phase::Working, false);
        let (a2, s2) = pulse_metrics(1100, 0, Phase::Working, false);
        assert!(a2 > a1 && s2 > s1);
        assert_eq!(pulse_metrics(999, 1, Phase::NeedsInput, true), (0.30, 1.06));
    }
}
