//! Whether a quota snapshot's plan is a provider's free tier.
//!
//! Adapters keep each provider's own plan wording (`free`, `Copilot Free`,
//! `KIRO FREE`, `personal_standard`, `Buddy 免费套餐`, ...). This is the one
//! rule both desktop UIs rely on when hiding free accounts' quota cards.
//! Unknown, missing and trial plans are never free, so a card is only hidden
//! when the provider itself reported a free tier.

use crate::models::QuotaSnapshot;

/// Provider codes that name a free tier without the word "free".
const FREE_CODES: &[&str] = &[
    // Qoder / Qoder CN `userType`: docs.qoder.cn/product-overview/account-and-subscription
    "personal_standard",
];

pub(crate) fn is_free(plan: Option<&str>) -> bool {
    let plan = plan.unwrap_or("").trim().to_lowercase();
    if plan.is_empty() {
        return false;
    }
    if FREE_CODES.contains(&plan.as_str()) {
        return true;
    }
    let words: Vec<&str> = plan
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    // A free trial of a paid tier (Cursor `free_trial` → "Cursor Pro Trial")
    // is still that paid tier. Raw strings: match tokens, not UI copy.
    if words.contains(&"trial") || plan.contains(r"试用") {
        return false;
    }
    words.contains(&"free") || plan.contains(r"免费")
}

/// Marks a snapshot before it leaves the core for a UI. Never persisted.
pub(crate) fn mark(q: &mut QuotaSnapshot) {
    q.free_plan = is_free(q.plan.as_deref());
}

#[cfg(test)]
mod tests {
    use super::is_free;

    #[test]
    fn provider_free_tiers_are_free() {
        for plan in [
            "free",                // Codex planType
            "Free",                // Antigravity, Gemini CLI, Trae
            " FREE ",              // padding and case
            "Copilot Free",        // Copilot
            "Cursor Free",         // Cursor stripeMembershipType
            "KIRO FREE",           // Kiro
            "free-tier",           // Google tier id
            "g1-free-tier",        // Google One license tier id
            "personal_standard",   // Qoder userType
            "Buddy 免费套餐",      // WorkBuddy / CodeBuddy
            "OpenRouter 免费额度", // OpenRouter key
            "免费",                // Trae CN display_desc
        ] {
            assert!(is_free(Some(plan)), "{plan}");
        }
    }

    #[test]
    fn paid_trial_and_unknown_plans_are_not_free() {
        for plan in [
            "",
            "   ",
            "plus",
            "pro",
            "go",
            "api",
            "unknown",
            "Copilot Pro",
            "Cursor Pro Trial",
            "free_trial",
            "Pro 试用",
            "Google AI Pro",
            "Standard",
            "standard-tier",
            "KIRO PRO+",
            "Kiro",
            "Trae",
            "personal_professional",
            "Buddy 付费套餐",
            "Buddy Enterprise",
            "OpenRouter",
            "Kilo 预付余额",
            "freedom", // only the whole word counts
        ] {
            assert!(!is_free(Some(plan)), "{plan}");
        }
        assert!(!is_free(None));
    }
}
