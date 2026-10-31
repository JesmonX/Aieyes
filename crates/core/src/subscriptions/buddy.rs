//! Buddy desktop billing schema observed in WorkBuddy 5.5.6. Only endpoints
//! that read balances are used: no check-in, purchase, renewal or account switch.
use crate::{
    buddy_auth::{self, Login},
    models::*,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

// The official desktop passes package codes to the paid/free endpoints.
const PAID: &[&str] = &[
    "TCACA_code_002_AkiJS3ZHF5",
    "TCACA_code_005_maRGyrHhw1",
    "TCACA_code_003_FAnt7lcmRT",
    "TCACA_code_023_4xbGhMrE6q",
    "TCACA_code_026_BaESVICNoi",
    "TCACA_code_027_0FCGVA6vSa",
    "TCACA_code_009_0XmEQc2xOf",
    "TCACA_code_038_OhvqZtiPKr",
    "TCACA_code_036_lupO5WgNdG",
];
const FREE: &[&str] = &[
    "TCACA_code_001_PqouKr6QWV",
    "TCACA_code_008_cfWoLwvjU4",
    "TCACA_code_035_ArVxJcGDsm",
    "TCACA_code_006_DbXS0lrypC",
    "TCACA_code_039_KRcQj7wUat",
    "TCACA_code_040_mi9rCYg46x",
    "TCACA_code_007_nzdH5h4Nl0",
    "TCACA_code_028_NtpWi0jzXs",
    "TCACA_code_037_WxOD3MpI2o",
    "TCACA_code_029_6wCGEWquYy",
    "TCACA_code_030_BjSt89qTvr",
];

pub(super) fn fetch(source: &Source, settings: &Settings, secret: &str) -> Result<Value> {
    let login: Login = serde_json::from_str(secret).context("Buddy 登录凭据格式无效")?;
    let base = buddy_auth::region_base(&login.region)?;
    fetch_at(source, settings, &login, base)
}
fn fetch_at(source: &Source, settings: &Settings, login: &Login, base: &str) -> Result<Value> {
    let client = buddy_auth::client(source.proxy.as_ref().unwrap_or(&settings.proxy))?;
    let post = |path: &str, body: &Value| -> Result<Value> {
        let v = buddy_auth::response(buddy_auth::authorized(
            client.post(format!("{base}{path}")).json(body),
            login,
        )?)?;
        Ok(buddy_auth::data(&v)?.clone())
    };
    if !login.enterprise.is_empty() {
        return Ok(
            json!({"enterprise":post("/v2/billing/meter/get-enterprise-user-usage", &json!({}))?}),
        );
    }
    let summary = post("/billing/meter/get-user-resource-summary", &json!({}))?;
    let mut body = json!({"summary":summary});
    for (kind, codes) in [("paid", PAID), ("free", FREE)] {
        let mut all = vec![];
        let mut page = 1;
        loop {
            crate::operations::check()?;
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let v = post(
                &format!("/billing/meter/get-user-resource-{kind}-packages"),
                &json!({"PageNumber":page,"PageSize":200,"PackageCodes":codes,"Status":[0,3],"NeedRenewInfo":true,"SlicePeriodStartTime":format!("{today} 00:00:00"),"SlicePeriodEndTime":format!("{today} 23:59:59")}),
            )?;
            let rows = v["Accounts"].as_array().context("Buddy 积分包字段缺失")?;
            let total = v["TotalCount"]
                .as_u64()
                .or_else(|| v["TotalCount"].as_str()?.parse().ok());
            all.extend(rows.iter().cloned());
            if total.is_some_and(|n| all.len() as u64 >= n) || total.is_none() && rows.len() < 200 {
                break;
            }
            ensure!(!rows.is_empty(), "Buddy 积分包响应不完整");
            ensure!(
                page < 100,
                "Buddy 积分包过多，未取得完整余额，请在官方客户端查看"
            );
            page += 1;
        }
        body[kind] = json!(all);
    }
    Ok(body)
}
fn count(v: &Value) -> Result<f64> {
    let n = v
        .as_f64()
        .or_else(|| v.as_str()?.parse::<f64>().ok())
        .context("Buddy 额度字段缺失")?;
    ensure!(n.is_finite() && n >= 0., "Buddy 额度格式无效");
    Ok(n)
}
fn stamp(v: &Value) -> Option<i64> {
    super::stamp(v).or_else(|| {
        chrono::NaiveDateTime::parse_from_str(v.as_str()?, "%Y-%m-%d %H:%M:%S")
            .ok()
            .and_then(|d| d.and_local_timezone(chrono::Local).earliest())
            .map(|d| d.timestamp())
    })
}
pub(super) fn parse(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    if let Some(e) = v.get("enterprise") {
        let used = count(&e["used_num"])?;
        let unlimited = e["limit_num"] == -1;
        let total = if unlimited {
            None
        } else {
            Some(count(&e["limit_num"])?)
        };
        q.meters.push(QuotaMeter {
            id: "buddy-enterprise".into(),
            label: "团队积分".into(),
            used,
            total,
            remaining: total.map(|n| (n - used).max(0.)),
            unit: "credits".into(),
            ..Default::default()
        });
        q.plan = Some("Buddy Enterprise".into());
        q.credits = Some(CreditsSnapshot {
            has_credits: true,
            unlimited,
            balance: total.map(|n| (n - used).max(0.).to_string()),
        });
        return Ok(());
    }
    let summary = &v["summary"];
    let packages = summary["Packages"]
        .as_array()
        .context("Buddy 余额汇总字段缺失")?;
    let mut remaining = 0.;
    for (i, p) in packages.iter().enumerate() {
        let (total, left, used) = (
            count(&p["CycleTotalCapacity"])?,
            count(&p["CycleRemainCapacity"])?,
            count(&p["CycleUsedCapacity"])?,
        );
        remaining += left;
        q.meters.push(QuotaMeter {
            id: format!("buddy-summary-{i}"),
            group: "Buddy 共享积分".into(),
            label: p["PackageName"].as_str().unwrap_or("套餐积分").into(),
            used,
            total: Some(total),
            remaining: Some(left),
            unit: "credits".into(),
            ..Default::default()
        });
    }
    q.credits = Some(CreditsSnapshot {
        has_credits: true,
        balance: Some(remaining.to_string()),
        ..Default::default()
    });
    q.plan = Some(
        if summary["IsPaidUser"] == true {
            "Buddy 付费套餐"
        } else {
            "Buddy 免费套餐"
        }
        .into(),
    );
    let mut seen = std::collections::HashSet::new();
    for (kind, label) in [("paid", "付费积分包"), ("free", "免费积分包")] {
        for p in v[kind].as_array().context("Buddy 积分包字段缺失")? {
            let id = p["ResourceId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("Buddy 积分包标识缺失")?;
            if !seen.insert(id.to_owned()) {
                continue;
            }
            let (total, left, reset) =
                if p["CapacityType"] == 4 && p["SlicePeriodUsageDetails"][0].is_object() {
                    let slice = &p["SlicePeriodUsageDetails"][0];
                    (
                        count(&slice["SlicePeriodCapacitySizePrecise"])?,
                        count(&slice["SlicePeriodCapacityRemainPrecise"])?,
                        stamp(&slice["SlicePeriodEndTime"]).or_else(|| stamp(&p["CycleEndTime"])),
                    )
                } else {
                    (
                        count(&p["CycleCapacitySizePrecise"])?,
                        count(&p["CycleCapacityRemainPrecise"])?,
                        stamp(&p["CycleEndTime"]).or_else(|| stamp(&p["DeductionEndTime"])),
                    )
                };
            q.meters.push(QuotaMeter {
                id: format!("buddy-package-{}", hash(id)),
                group: label.into(),
                label: p["PackageName"]
                    .as_str()
                    .unwrap_or(label)
                    .chars()
                    .take(128)
                    .collect(),
                used: (total - left).max(0.),
                total: Some(total),
                remaining: Some(left),
                unit: "credits".into(),
                resets_at: reset,
                ..Default::default()
            });
        }
    }
    q.notice = Some("CodeBuddy / WorkBuddy 共用余额；本月已用为本机记录。".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../../../../scripts/fixtures/buddy-billing.json"
        ))
        .unwrap()
    }
    #[test]
    fn buddy_balance_uses_summary_once_and_retains_paid_free_and_slice_details() {
        let mut v = fixture();
        let duplicate = v["paid"][0].clone();
        v["free"].as_array_mut().unwrap().push(duplicate);
        let mut q = QuotaSnapshot::default();
        parse(&mut q, &v).unwrap();
        assert_eq!(q.credits.unwrap().balance.as_deref(), Some("3625"));
        assert_eq!(q.meters.len(), 4);
        assert_eq!(q.meters[3].remaining, Some(125.));
        assert!(q.meters[3].resets_at.is_some());
        v["free"][0]["SlicePeriodUsageDetails"] =
            json!([{"SlicePeriodCapacitySizePrecise":50,"SlicePeriodCapacityRemainPrecise":30}]);
        let mut q = QuotaSnapshot::default();
        parse(&mut q, &v).unwrap();
        assert_eq!(q.meters[3].remaining, Some(30.));
        v["summary"]["Packages"][0]["CycleTotalCapacity"] = json!("NaN");
        assert!(parse(&mut QuotaSnapshot::default(), &v).is_err());
        assert!(parse(&mut QuotaSnapshot::default(), &json!({})).is_err());
    }
    #[test]
    fn buddy_enterprise_unlimited_is_not_zero_remaining() {
        let mut q = QuotaSnapshot::default();
        parse(
            &mut q,
            &json!({"enterprise":{"used_num":250,"limit_num":-1}}),
        )
        .unwrap();
        assert!(q.credits.unwrap().unlimited);
        assert_eq!(q.meters[0].total, None);
        assert_eq!(q.meters[0].used, 250.);
    }
    #[test]
    fn buddy_billing_fetches_each_page_and_uses_aieyes_token_headers() {
        let fixture = fixture();
        let rows: Vec<_> = (0..200)
            .map(|i| json!({"ResourceId":format!("paid-{i}")}))
            .collect();
        let (base, server) = crate::buddy_auth::tests::mock(vec![
            json!({"code":0,"data":fixture["summary"]}),
            json!({"code":0,"data":{"Accounts":rows,"TotalCount":201}}),
            json!({"code":0,"data":{"Accounts":fixture["paid"],"TotalCount":201}}),
            json!({"code":0,"data":{"Accounts":fixture["free"],"TotalCount":1}}),
        ]);
        let settings = Settings {
            proxy: ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let body = fetch_at(
            &Source::default(),
            &settings,
            &crate::buddy_auth::tests::login(),
            &base,
        )
        .unwrap();
        assert_eq!(body["paid"].as_array().unwrap().len(), 201);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[2].contains("\"PageNumber\":2"));
        assert!(
            requests
                .iter()
                .all(|r| r.to_lowercase().contains("x-user-id: fixture-user"))
        );
        assert!(requests.iter().all(|r| {
            r.to_lowercase()
                .contains("authorization: bearer buddy_fixture_secret")
        }));
        assert!(requests[3].contains("TCACA_code_008_cfWoLwvjU4"));
    }
}
