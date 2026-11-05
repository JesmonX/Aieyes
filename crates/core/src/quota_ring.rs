//! The quota ring: one account drawn as a circle. The top half is the short
//! window (5h), the bottom half the long one (7d). Colored arcs are the
//! remaining share. Both arcs start at the left and grow to the right, so the
//! halves mirror each other and compare at a glance.
//!
//! The macOS app (`QuotaRingCore.swift`) and the Web UI (`quota-rings.js`) apply
//! the same rules to their tiles. The tray and the menu bar use [`menu_ring`].

use crate::models::{MenuQuotaRing, QuotaSnapshot, QuotaWindow, Settings};
use serde::Serialize;

/// Labels that can be shown beside the menu bar / tray ring, in display order.
pub const MENU_VALUES: [&str; 4] = ["topRemaining", "topReset", "bottomRemaining", "bottomReset"];

pub const FIVE_HOURS: i64 = 300;
pub const WEEK: i64 = 10_080;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Arc {
    /// Remaining share, 0–100.
    pub remaining: f64,
    pub resets_at: Option<i64>,
    pub window_minutes: Option<i64>,
    /// The window reported no use yet (used 0 and no reset time).
    pub unused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Ring {
    /// Two windows: the top and bottom halves.
    Duo { top: Arc, bottom: Arc },
    /// One pool: a full circle.
    Single { top: Arc },
    /// A balance without a total: a filled coin, never a share.
    Coin { amount: String, low: bool },
    /// No reading or a failed query.
    Empty,
}

impl Ring {
    pub fn top(&self) -> Option<&Arc> {
        match self {
            Ring::Duo { top, .. } | Ring::Single { top } => Some(top),
            _ => None,
        }
    }
    pub fn bottom(&self) -> Option<&Arc> {
        match self {
            Ring::Duo { bottom, .. } => Some(bottom),
            _ => None,
        }
    }
}

fn arc(window: &QuotaWindow) -> Arc {
    let used = if window.used_percent.is_finite() {
        window.used_percent.clamp(0.0, 100.0)
    } else {
        100.0
    };
    Arc {
        remaining: 100.0 - used,
        resets_at: window.resets_at.filter(|stamp| *stamp > 0),
        window_minutes: window.window_minutes,
        unused: used == 0.0 && window.resets_at.is_none_or(|stamp| stamp <= 0),
    }
}

/// Any provider whose windows share a group name is grouped (Antigravity models);
/// each group gets its own ring.
pub fn groups_windows(quota: &QuotaSnapshot) -> bool {
    if quota.provider == "codex" {
        return false;
    }
    if quota.provider == "antigravity" {
        return true;
    }
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for window in quota.windows.iter().filter(|w| !w.group_name.is_empty()) {
        *counts.entry(&window.group_name).or_default() += 1;
    }
    counts.values().any(|count| *count > 1)
}

fn group_label(window: &QuotaWindow) -> String {
    if !window.group_name.is_empty() {
        return window.group_name.clone();
    }
    let parts: Vec<_> = window.name.split(" · ").collect();
    parts[..parts.len().saturating_sub(1)].join(" · ")
}

/// The windows of the first group, or all windows when the quota is not grouped.
fn ring_windows(quota: &QuotaSnapshot) -> Vec<&QuotaWindow> {
    if !groups_windows(quota) {
        return quota.windows.iter().collect();
    }
    let Some(first) = quota.windows.first().map(group_label) else {
        return vec![];
    };
    quota
        .windows
        .iter()
        .filter(|w| group_label(w) == first)
        .collect()
}

/// Top and bottom windows. A 5h + 7d pair wins; otherwise two windows of
/// different known lengths, shorter on top; otherwise the first window alone.
pub fn pick<'a>(windows: &[&'a QuotaWindow]) -> (Option<&'a QuotaWindow>, Option<&'a QuotaWindow>) {
    let five = windows
        .iter()
        .find(|w| w.window_minutes == Some(FIVE_HOURS));
    let week = windows.iter().find(|w| w.window_minutes == Some(WEEK));
    if let (Some(top), Some(bottom)) = (five, week) {
        return (Some(top), Some(bottom));
    }
    let mut timed: Vec<_> = windows
        .iter()
        .filter(|w| w.window_minutes.is_some_and(|m| m > 0))
        .collect();
    timed.sort_by_key(|w| w.window_minutes);
    if let Some(top) = timed.first()
        && let Some(bottom) = timed
            .iter()
            .find(|w| w.window_minutes != top.window_minutes)
    {
        return (Some(top), Some(bottom));
    }
    (windows.first().copied(), None)
}

fn currency(code: &str) -> String {
    match code.trim().to_ascii_uppercase().as_str() {
        "CNY" | "RMB" => "¥".into(),
        "USD" => "$".into(),
        "" => String::new(),
        other => other.to_string() + " ",
    }
}

fn meter_amount(unit: &str, value: f64) -> String {
    let text = if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    };
    match unit {
        "usd" => format!("${text}"),
        "cny" => format!("¥{text}"),
        _ => text,
    }
}

pub fn ring(quota: &QuotaSnapshot) -> Ring {
    let windows = ring_windows(quota);
    match pick(&windows) {
        (Some(top), Some(bottom)) => {
            return Ring::Duo {
                top: arc(top),
                bottom: arc(bottom),
            };
        }
        (Some(top), None) => return Ring::Single { top: arc(top) },
        _ => {}
    }
    if let Some(meter) = quota.meters.iter().find(|m| {
        m.used_known != Some(false) && m.total.is_some_and(|t| t > 0.0) && m.used.is_finite()
    }) {
        let fraction = (meter.used / meter.total.unwrap_or(1.0)).clamp(0.0, 1.0);
        return Ring::Single {
            top: Arc {
                remaining: 100.0 - fraction * 100.0,
                resets_at: meter.resets_at.filter(|stamp| *stamp > 0),
                window_minutes: None,
                unused: false,
            },
        };
    }
    let low = quota.is_available == Some(false);
    if let Some(balance) = quota.balances.first() {
        return Ring::Coin {
            amount: currency(&balance.currency) + balance.total.trim(),
            low,
        };
    }
    if let Some(wallet) = &quota.extra_usage {
        return Ring::Coin {
            amount: currency(&wallet.currency) + wallet.balance.as_deref().unwrap_or(&wallet.total),
            low,
        };
    }
    if let Some(meter) = quota.meters.iter().find(|m| m.remaining.is_some()) {
        return Ring::Coin {
            amount: meter_amount(&meter.unit, meter.remaining.unwrap_or_default()),
            low,
        };
    }
    if let Some(balance) = quota.credits.as_ref().and_then(|c| c.balance.clone()) {
        return Ring::Coin {
            amount: balance,
            low,
        };
    }
    Ring::Empty
}

/// `47m`, `2h13m`, `14h`, `1d5h`, `3d`: unit letters only, so every language reads it.
pub fn compact_minutes(minutes: i64) -> String {
    let minutes = minutes.max(1);
    if minutes < 60 {
        return format!("{minutes}m");
    }
    if minutes < 600 {
        let (hours, rest) = (minutes / 60, minutes % 60);
        return if rest == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h{rest}m")
        };
    }
    if minutes < 1440 {
        return format!("{}h", (minutes as f64 / 60.0).round() as i64);
    }
    if minutes < 4320 {
        let (days, hours) = (
            minutes / 1440,
            ((minutes % 1440) as f64 / 60.0).round() as i64,
        );
        return if hours == 0 {
            format!("{days}d")
        } else if hours == 24 {
            format!("{}d", days + 1)
        } else {
            format!("{days}d{hours}h")
        };
    }
    format!("{}d", (minutes as f64 / 1440.0).round() as i64)
}

/// Time until the arc's next reset. An unused 5h/7d window shows its length,
/// a passed reset `…` (the next reading confirms it), an unknown one `—`.
pub fn reset_text(arc: &Arc, now: i64) -> String {
    if arc.unused {
        match arc.window_minutes {
            Some(FIVE_HOURS) => return "5h".into(),
            Some(WEEK) => return "7d".into(),
            _ => {}
        }
    }
    match arc.resets_at {
        Some(stamp) if stamp > now => compact_minutes((stamp - now + 59) / 60),
        Some(_) => "…".into(),
        None => "—".into(),
    }
}

pub fn remaining_text(arc: &Arc) -> String {
    format!("{:.0}%", arc.remaining)
}

/// Warn at 30% remaining or less, danger under 10% (the cards' thresholds).
pub fn level(remaining: f64) -> &'static str {
    if remaining < 10.0 {
        "danger"
    } else if remaining <= 30.0 {
        "warn"
    } else {
        "ok"
    }
}

/// The text beside the ring for the configured values; missing ones are skipped.
pub fn label(ring: &Ring, values: &[String], now: i64) -> String {
    values
        .iter()
        .filter_map(|value| match (value.as_str(), ring) {
            ("topRemaining", Ring::Coin { amount, .. }) => Some(amount.clone()),
            ("topRemaining", _) => ring.top().map(remaining_text),
            ("topReset", _) => ring.top().map(|arc| reset_text(arc, now)),
            ("bottomRemaining", _) => ring.bottom().map(remaining_text),
            ("bottomReset", _) => ring.bottom().map(|arc| reset_text(arc, now)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuRing {
    pub provider: String,
    pub name: String,
    pub ring: Ring,
    pub label: String,
}

/// The first shown account (quota order, free plans hidden when the user asked)
/// as the menu bar / tray ring. `quotas` must already be in display order.
pub fn menu_ring(settings: &Settings, quotas: &[QuotaSnapshot], now: i64) -> Option<MenuRing> {
    let MenuQuotaRing { enabled, values } = &settings.menu_quota_ring;
    if !enabled {
        return None;
    }
    let quota = quotas
        .iter()
        .find(|q| !(settings.hide_free_quota_cards && q.free_plan))?;
    let ring = ring(quota);
    if ring == Ring::Empty {
        return None;
    }
    Some(MenuRing {
        provider: quota.provider.clone(),
        name: quota.name.clone(),
        label: label(&ring, values, now),
        ring,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Balance, QuotaMeter};

    fn window(name: &str, used: f64, minutes: Option<i64>, reset: Option<i64>) -> QuotaWindow {
        QuotaWindow {
            name: name.into(),
            used_percent: used,
            window_minutes: minutes,
            resets_at: reset,
            ..Default::default()
        }
    }

    #[test]
    fn five_hour_and_week_windows_become_the_two_halves() {
        let quota = QuotaSnapshot {
            provider: "kimi".into(),
            windows: vec![
                window("月度", 12.0, Some(43_200), Some(9_000)),
                window("7d", 42.0, Some(WEEK), Some(4_000)),
                window("5h", 35.0, Some(FIVE_HOURS), Some(2_000)),
            ],
            ..Default::default()
        };
        let Ring::Duo { top, bottom } = ring(&quota) else {
            panic!("expected two halves")
        };
        assert_eq!((top.remaining, bottom.remaining), (65.0, 58.0));
        assert_eq!(top.resets_at, Some(2_000));
    }

    #[test]
    fn windows_of_one_length_share_a_single_circle() {
        let quota = QuotaSnapshot {
            provider: "cursor".into(),
            windows: vec![
                window("Cursor", 36.0, None, Some(10)),
                window("其他", 12.0, None, None),
            ],
            ..Default::default()
        };
        assert!(matches!(ring(&quota), Ring::Single { top } if top.remaining == 64.0));
    }

    #[test]
    fn grouped_windows_use_the_first_group() {
        let quota = QuotaSnapshot {
            provider: "antigravity".into(),
            windows: vec![
                QuotaWindow {
                    group_name: "Claude".into(),
                    ..window("Claude · 5h", 28.0, Some(FIVE_HOURS), None)
                },
                QuotaWindow {
                    group_name: "Claude".into(),
                    ..window("Claude · 7d", 28.0, Some(WEEK), None)
                },
                QuotaWindow {
                    group_name: "Gemini".into(),
                    ..window("Gemini · 5h", 65.0, Some(FIVE_HOURS), None)
                },
            ],
            ..Default::default()
        };
        assert!(matches!(ring(&quota), Ring::Duo { top, .. } if top.remaining == 72.0));
    }

    #[test]
    fn balances_are_coins_and_meters_with_totals_are_circles() {
        let coin = QuotaSnapshot {
            provider: "deepseek".into(),
            balances: vec![Balance {
                currency: "CNY".into(),
                total: "38.20".into(),
                ..Default::default()
            }],
            is_available: Some(false),
            ..Default::default()
        };
        assert_eq!(
            ring(&coin),
            Ring::Coin {
                amount: "¥38.20".into(),
                low: true
            }
        );
        let meter = QuotaSnapshot {
            provider: "copilot".into(),
            meters: vec![QuotaMeter {
                id: "p".into(),
                used: 75.0,
                total: Some(300.0),
                unit: "requests".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(matches!(ring(&meter), Ring::Single { top } if top.remaining == 75.0));
        assert_eq!(ring(&QuotaSnapshot::default()), Ring::Empty);
    }

    #[test]
    fn compact_times_keep_two_units_at_most() {
        let cases = [
            (1, "1m"),
            (47, "47m"),
            (60, "1h"),
            (133, "2h13m"),
            (599, "9h59m"),
            (840, "14h"),
            (1440, "1d"),
            (1740, "1d5h"),
            (4319, "3d"),
            (4560, "3d"),
            (8640, "6d"),
        ];
        for (minutes, text) in cases {
            assert_eq!(compact_minutes(minutes), text, "{minutes}");
        }
    }

    #[test]
    fn labels_follow_the_configured_values() {
        let now = 1_000_000;
        let ring = Ring::Duo {
            top: Arc {
                remaining: 64.6,
                resets_at: Some(now + 133 * 60),
                window_minutes: Some(FIVE_HOURS),
                unused: false,
            },
            bottom: Arc {
                remaining: 58.0,
                resets_at: None,
                window_minutes: Some(WEEK),
                unused: true,
            },
        };
        let values = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            label(&ring, &values(&["topRemaining", "topReset"]), now),
            "65% 2h13m"
        );
        assert_eq!(label(&ring, &values(&["bottomReset"]), now), "7d");
        assert_eq!(label(&ring, &values(&[]), now), "");
        let passed = Ring::Single {
            top: Arc {
                remaining: 0.0,
                resets_at: Some(now - 5),
                window_minutes: None,
                unused: false,
            },
        };
        assert_eq!(
            label(&passed, &values(&["topReset", "bottomReset"]), now),
            "…"
        );
    }

    #[test]
    fn settings_keep_two_known_values() {
        let mut setting = MenuQuotaRing {
            enabled: true,
            values: [
                "topReset",
                "nope",
                "topReset",
                "bottomRemaining",
                "bottomReset",
            ]
            .map(String::from)
            .to_vec(),
        };
        setting.normalize();
        assert_eq!(setting.values, ["topReset", "bottomRemaining"]);
    }
}
