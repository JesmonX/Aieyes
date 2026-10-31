//! Trae / Trae CN usage through `ide_user_ent_usage`, the non-public call behind
//! the IDE's usage panel. The request, headers and field names follow the
//! installed clients (Trae 3.6.4, Trae CN 3.4.11); responses stay unverified
//! until a real account is checked (docs/agent-subscriptions.md).
//!
//! Both clients keep their login (`iCubeAuthInfo://icube.cloudide` in
//! `User/globalStorage/storage.json`) in an app-encrypted envelope, which Aieyes
//! never decrypts. Only a plain-text login is used, read at query time, never
//! refreshed, and no device ID is sent or changed.
use super::*;

pub(super) const RELOGIN: &str = "Trae 登录已失效，打开 Trae 一次即可刷新";
const UNSUPPORTED: &str = "Trae 额度响应格式不支持";

/// Trae and Trae CN: separate account systems, one adapter.
pub(crate) fn family(provider: &str) -> bool {
    matches!(provider, "trae" | "trae-cn")
}

/// The edition's default `ug` API host (product.json `bootConfig.ug`) and path.
pub(super) fn default_url(provider: &str) -> &'static str {
    if provider == "trae-cn" {
        "https://api.trae.cn/trae/api/v2/pay/ide_user_ent_usage"
    } else {
        "https://grow-normal.trae.ai/trae/api/v1/pay/ide_user_ent_usage"
    }
}

/// `https://<host>` when the host belongs to the edition's own domains.
pub(crate) fn api_origin(provider: &str, host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('/');
    let host = host.strip_prefix("https://").unwrap_or(host);
    let domains: &[&str] = if provider == "trae-cn" {
        &[".trae.cn", ".trae.com.cn"]
    } else {
        &[".trae.ai", ".traeapi.us"]
    };
    let ok = !host.is_empty()
        && host.len() <= 253
        && domains.iter().any(|d| host.ends_with(d))
        && host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.');
    ok.then(|| format!("https://{host}"))
}

fn amount(v: &Value) -> Option<f64> {
    let n = number(v).ok()?;
    (n.is_finite() && n >= 0.).then_some(n)
}

/// Epoch seconds from a number (s or ms) or an RFC 3339 string.
fn instant(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_f64().filter(|n| *n > 0.) {
        let n = n as i64;
        return Some(if n > 10_000_000_000 { n / 1000 } else { n });
    }
    chrono::DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

// `entitlement_base_info.product_type` and pack `status`, as the clients define them.
const PROMO_CODE: i64 = 3;
const PAY_GO: i64 = 7;
const TERMINATED: i64 = 4;
const EXPIRED: i64 = 5;
/// Plans in the order the clients pick the one shown as the account's plan.
const PLANS: &[(i64, &str)] = &[
    (100, "Express"),
    (6, "Ultra"),
    (4, "Pro+"),
    (1, "Pro"),
    (8, "Lite"),
    (0, "Free"),
];
/// (limit field, used field, meter id, unit, label). A limit of -1 is unlimited.
const KINDS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "credits_limit",
        "credits_amount",
        "credits",
        "credits",
        "Credits",
    ),
    (
        "basic_usage_limit",
        "basic_usage_amount",
        "basic",
        "usd",
        "基础用量",
    ),
    (
        "bonus_usage_limit",
        "bonus_usage_amount",
        "bonus",
        "usd",
        "赠送用量",
    ),
    (
        "premium_model_fast_request_limit",
        "premium_model_fast_amount",
        "fast",
        "requests",
        "快速请求",
    ),
];

/// One meter per quota of each active entitlement pack: Credits on credits
/// billing (Trae CN, Work), dollars on usage billing, and the legacy fast-request
/// count otherwise; pay-as-you-go spend has no limit. Promo codes, ended,
/// expired and terminated packs are skipped, as in the IDE.
pub(super) fn parse(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    ensure!(v["code"].is_null() || v["code"] == 0, "Trae 额度查询被拒绝");
    let v = if v["data"].is_object() { &v["data"] } else { v };
    ensure!(v["code"].is_null() || v["code"] == 0, "Trae 额度查询被拒绝");
    let dollars = v["is_dollar_usage_billing"].as_bool() == Some(true);
    let credits = v["is_credits_billing"].as_bool() == Some(true);
    let packs = v["user_entitlement_pack_list"]
        .as_array()
        .context(UNSUPPORTED)?;
    let mut plans = vec![];
    for (i, pack) in packs.iter().enumerate() {
        let base = &pack["entitlement_base_info"];
        let product = base["product_type"].as_i64();
        let ends = instant(&base["end_time"]);
        if product == Some(PROMO_CODE)
            || matches!(pack["status"].as_i64(), Some(TERMINATED | EXPIRED))
            || ends.is_some_and(|t| t <= now())
        {
            continue;
        }
        let tier = product.and_then(|p| PLANS.iter().find(|(id, _)| *id == p));
        let name = label(&pack["display_desc"]).or_else(|| tier.map(|(_, n)| (*n).to_owned()));
        if let Some((id, _)) = tier {
            plans.push((*id, name.clone()));
        }
        let quota = &base["quota"];
        let usage = &pack["usage"];
        // The IDE shows the next reset one second after the period ends.
        let resets = ends.map(|t| t + 1);
        for (limit, used, id, unit, title) in KINDS {
            let wanted = match *unit {
                "credits" => credits || !dollars,
                "usd" => dollars || !credits,
                _ => !dollars && !credits,
            };
            if !wanted || (*id == "bonus" && quota["no_bonus_quota"] == true) {
                continue;
            }
            let Ok(limit) = number(&quota[*limit]) else {
                continue;
            };
            let total = if limit == -1. {
                None
            } else if limit > 0. {
                Some(limit)
            } else {
                continue;
            };
            let spent = amount(&usage[*used])
                .or_else(|| {
                    (*id == "fast")
                        .then(|| amount(&usage["premium_model_fast_request_usage"]))
                        .flatten()
                })
                .unwrap_or(0.);
            q.meters.push(QuotaMeter {
                id: format!("pack-{i}-{id}"),
                group: name.clone().unwrap_or_default(),
                label: (*title).into(),
                used: spent,
                total,
                remaining: total.map(|t| (t - spent).max(0.)),
                unit: (*unit).into(),
                resets_at: resets,
                ..Default::default()
            });
        }
        if product == Some(PAY_GO)
            && let Some(spent) = amount(&usage["pay_go_amount"])
        {
            q.meters.push(QuotaMeter {
                id: format!("pack-{i}-pay-go"),
                group: name.clone().unwrap_or_default(),
                label: "按量付费".into(),
                used: spent,
                unit: "usd".into(),
                resets_at: resets,
                ..Default::default()
            });
        }
    }
    q.plan = PLANS
        .iter()
        .find_map(|(id, _)| plans.iter().find(|(p, _)| p == id))
        .and_then(|(_, name)| name.clone())
        .or_else(|| {
            Some(
                if q.provider == "trae-cn" {
                    "Trae CN"
                } else {
                    "Trae"
                }
                .into(),
            )
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures follow the clients' field names; the live schema is unverified.
    #[test]
    fn dollar_billing_becomes_usd_meters_and_inactive_packs_are_skipped() {
        let future = now() + 86400;
        let v = json!({
        "is_dollar_usage_billing":true,"is_credits_billing":false,
        "user_entitlement_pack_list":[
            {"entitlement_base_info":{"product_type":1,"end_time":future,
                "quota":{"basic_usage_limit":20,"bonus_usage_limit":5,"premium_model_fast_request_limit":600}},
             "usage":{"basic_usage_amount":7.5,"bonus_usage_amount":0},"status":1},
            {"entitlement_base_info":{"product_type":0,"end_time":1,"quota":{"basic_usage_limit":99}},"usage":{}},
            {"entitlement_base_info":{"product_type":3,"end_time":future,"quota":{"basic_usage_limit":50}},"usage":{}},
            {"entitlement_base_info":{"product_type":6,"end_time":future,"quota":{"basic_usage_limit":50}},"usage":{},"status":5},
            {"entitlement_base_info":{"product_type":7,"end_time":future,"quota":{}},"usage":{"pay_go_amount":1.25}}
        ]});
        let mut q = QuotaSnapshot {
            provider: "trae".into(),
            ..Default::default()
        };
        parse(&mut q, &v).unwrap();
        let ids: Vec<_> = q.meters.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["pack-0-basic", "pack-0-bonus", "pack-4-pay-go"]);
        assert_eq!(q.meters[0].unit, "usd");
        assert_eq!(q.meters[0].group, "Pro");
        assert_eq!(q.meters[0].remaining, Some(12.5));
        assert_eq!(q.meters[0].resets_at, Some(future + 1));
        assert_eq!((q.meters[2].used, q.meters[2].total), (1.25, None));
        assert!(q.meters.iter().all(QuotaMeter::valid));
        assert_eq!(q.plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn credits_billing_unlimited_and_legacy_fast_requests() {
        let v = json!({"code":0,"is_credits_billing":true,"user_entitlement_pack_list":[
            {"entitlement_base_info":{"product_type":100,"quota":{"credits_limit":3000}},"usage":{"credits_amount":1200},"display_desc":"专业版"},
            {"entitlement_base_info":{"product_type":0,"quota":{"credits_limit":-1}},"usage":{"credits_amount":4}}]});
        let mut q = QuotaSnapshot {
            provider: "trae-cn".into(),
            ..Default::default()
        };
        parse(&mut q, &v).unwrap();
        assert_eq!(q.meters.len(), 2);
        assert_eq!(
            (q.meters[0].unit.as_str(), q.meters[0].remaining),
            ("credits", Some(1800.))
        );
        assert_eq!((q.meters[1].total, q.meters[1].remaining), (None, None));
        assert_eq!(q.plan.as_deref(), Some("专业版"));
        let legacy = json!({"user_entitlement_pack_list":[{"entitlement_base_info":{"quota":{"premium_model_fast_request_limit":600}},"usage":{"premium_model_fast_request_usage":12}}]});
        let mut q = QuotaSnapshot {
            provider: "trae".into(),
            ..Default::default()
        };
        parse(&mut q, &legacy).unwrap();
        assert_eq!(
            (q.meters[0].unit.as_str(), q.meters[0].used),
            ("requests", 12.)
        );
        assert_eq!(q.plan.as_deref(), Some("Trae"));
        assert!(parse(&mut QuotaSnapshot::default(), &json!({"x":1})).is_err());
        assert!(
            parse(
                &mut QuotaSnapshot::default(),
                &json!({"code":1001,"data":{"user_entitlement_pack_list":[]}})
            )
            .is_err()
        );
        assert!(
            parse(
                &mut QuotaSnapshot::default(),
                &json!({"code":1001,"user_entitlement_pack_list":[]})
            )
            .is_err()
        );
    }

    #[test]
    fn only_the_editions_own_hosts_receive_the_token() {
        assert_eq!(
            api_origin("trae", "growsg-normal.trae.ai").as_deref(),
            Some("https://growsg-normal.trae.ai")
        );
        assert_eq!(
            api_origin("trae", "grow-normal.traeapi.us").as_deref(),
            Some("https://grow-normal.traeapi.us")
        );
        assert_eq!(
            api_origin("trae-cn", "https://api.trae.cn/").as_deref(),
            Some("https://api.trae.cn")
        );
        assert!(api_origin("trae", "api.trae.cn").is_none());
        assert!(api_origin("trae-cn", "grow-normal.trae.ai").is_none());
        assert!(api_origin("trae", "evil.example/x.trae.ai").is_none());
        assert!(api_origin("trae", "trae.ai.evil.example").is_none());
        assert!(default_url("trae-cn").starts_with("https://api.trae.cn/trae/api/v2/"));
        assert!(default_url("trae").starts_with("https://grow-normal.trae.ai/trae/api/v1/"));
    }
}
