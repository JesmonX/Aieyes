//! Kiro credits through `GetUsageLimits`, the call behind kiro-cli's `/usage`.
//! The token is read from kiro-cli's database or the IDE token file and never
//! refreshed: SSO refresh tokens rotate, so refreshing would sign kiro-cli out.
//! Only an expired token falls back to running `kiro-cli /usage` itself.
//! Field meanings follow the usage views of Kiro IDE 1.4.62 and kiro-cli
//! 2.36.0; live responses are untested (no Kiro account), see
//! docs/agent-subscriptions.md.
use super::*;
use chrono::Datelike;

pub(super) const TARGET: &str = "AmazonCodeWhispererService.GetUsageLimits";
pub(crate) const RELOGIN: &str = "Kiro 登录已失效，运行 kiro-cli 或打开 Kiro 一次即可刷新";
/// Seconds between two fallback runs of `kiro-cli /usage`, whatever the schedule.
pub(crate) const CLI_FLOOR: i64 = 300;
/// The official clients treat a limit at or above this as unlimited.
const UNLIMITED: f64 = 999_999.;

/// Regions with a known endpoint; the profile ARN decides which one is used.
pub(super) fn region_url(arn: &str) -> Result<&'static str> {
    if arn.is_empty() {
        return Ok("https://codewhisperer.us-east-1.amazonaws.com/");
    }
    let mut parts = arn.split(':');
    ensure!(
        parts.next() == Some("arn")
            && parts.next() == Some("aws")
            && parts.next() == Some("codewhisperer")
            && arn.len() <= 256
            && !arn.chars().any(char::is_control),
        "Kiro 配置格式不支持"
    );
    match parts.next() {
        Some("us-east-1") => Ok("https://codewhisperer.us-east-1.amazonaws.com/"),
        Some("eu-central-1") => Ok("https://q.eu-central-1.amazonaws.com/"),
        _ => bail!("此 Kiro 区域暂不支持额度查询"),
    }
}

fn credits(v: &Value, precise: &str, plain: &str) -> Option<f64> {
    let n = number(&v[precise]).or_else(|_| number(&v[plain])).ok()?;
    (n.is_finite() && n >= 0.).then_some(n)
}

/// Epoch seconds from a number (s or ms) or an RFC 3339 string.
fn instant(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        let n = n as i64;
        return Some(if n > 10_000_000_000 { n / 1000 } else { n });
    }
    chrono::DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

/// A status the official clients count; a missing status counts too.
fn counted(status: &Value, allowed: &[&str]) -> bool {
    status
        .as_str()
        .is_none_or(|s| allowed.iter().any(|a| s.eq_ignore_ascii_case(a)))
}

/// The numbers live in the CREDIT entry of `usageBreakdownList` (or, in older
/// shapes, at the top level). `currentUsage` includes overage, which gets its
/// own meter; the trial, bonuses and add-on packs are separate pools.
pub(super) fn parse(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let list = v["usageBreakdownList"].as_array();
    let entry = list
        .and_then(|l| {
            l.iter()
                .find(|e| text(&e["resourceType"]).eq_ignore_ascii_case("credit"))
                .or_else(|| l.first())
        })
        .unwrap_or(v);
    let reset = instant(&entry["nextDateReset"]).or_else(|| instant(&v["nextDateReset"]));
    let total = credits(entry, "currentUsageWithPrecision", "currentUsage");
    let limit = credits(entry, "usageLimitWithPrecision", "usageLimit");
    ensure!(
        total.is_some() || limit.is_some(),
        "Kiro 额度响应格式不支持"
    );
    let overage = credits(entry, "currentOveragesWithPrecision", "currentOverages").unwrap_or(0.);
    let used = (total.unwrap_or(0.) - overage).max(0.);
    let limit = limit.filter(|l| *l > 0. && *l < UNLIMITED);
    q.meters.push(QuotaMeter {
        id: "plan".into(),
        label: "套餐 Credits".into(),
        used,
        total: limit,
        remaining: limit.map(|l| (l - used).max(0.)),
        unit: "credits".into(),
        resets_at: reset,
        ..Default::default()
    });
    let disabled =
        text(&v["overageConfiguration"]["overageStatus"]).eq_ignore_ascii_case("disabled");
    let cap =
        credits(entry, "overageCapWithPrecision", "overageCap").filter(|c| *c > 0. && !disabled);
    if overage > 0. || cap.is_some() {
        q.meters.push(QuotaMeter {
            id: "overage".into(),
            label: "超额 Credits".into(),
            used: overage,
            total: cap,
            remaining: cap.map(|c| (c - overage).max(0.)),
            unit: "credits".into(),
            resets_at: reset,
            ..Default::default()
        });
    }
    let trial = &entry["freeTrialInfo"];
    let trial = (trial.is_object() && counted(&trial["freeTrialStatus"], &["ACTIVE"]))
        .then_some(("trial".to_owned(), trial));
    let bonuses = entry["bonuses"]
        .as_array()
        .or(v["bonuses"].as_array())
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, b)| counted(&b["status"], &["ACTIVE", "EXHAUSTED"]))
        .map(|(i, b)| (format!("bonus-{i}"), b));
    for (id, bonus) in bonuses.chain(trial) {
        let Some(total) = credits(bonus, "usageLimitWithPrecision", "usageLimit") else {
            continue;
        };
        let used = credits(bonus, "currentUsageWithPrecision", "currentUsage").unwrap_or(0.);
        let expires = instant(&bonus["expiresAt"])
            .or_else(|| instant(&bonus["freeTrialExpiry"]))
            .or_else(|| instant(&bonus["expiry"]));
        if expires.is_some_and(|t| t <= now()) {
            continue;
        }
        let name = label(&bonus["displayName"])
            .or_else(|| label(&bonus["bonusCode"]))
            .unwrap_or_else(|| {
                if id == "trial" {
                    "试用 Credits".into()
                } else {
                    "赠送 Credits".into()
                }
            });
        q.meters.push(QuotaMeter {
            id,
            group: "赠送".into(),
            label: name,
            used,
            total: Some(total),
            remaining: Some((total - used).max(0.)),
            unit: "credits".into(),
            resets_at: expires,
            ..Default::default()
        });
    }
    // Purchased add-on packs, shown as one pool like the official `/usage`.
    let packs: Vec<_> = entry["overageCredits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| instant(&p["expiresAt"]).is_none_or(|t| t > now()))
        .filter_map(|p| {
            let total = credits(p, "usageLimitWithPrecision", "usageLimit")?;
            let used = credits(p, "currentUsageWithPrecision", "currentUsage").unwrap_or(0.);
            Some((used, total, instant(&p["expiresAt"])))
        })
        .collect();
    if !packs.is_empty() {
        let used: f64 = packs.iter().map(|p| p.0).sum();
        let total: f64 = packs.iter().map(|p| p.1).sum();
        q.meters.push(QuotaMeter {
            id: "add-on".into(),
            label: "加购 Credits".into(),
            used,
            total: Some(total),
            remaining: Some((total - used).max(0.)),
            unit: "credits".into(),
            resets_at: packs.iter().filter_map(|p| p.2).min(),
            ..Default::default()
        });
    }
    q.plan = label(&v["subscriptionInfo"]["subscriptionTitle"])
        .or_else(|| label(&v["subscriptionInfo"]["type"]))
        .or_else(|| Some("Kiro".into()));
    let user = &v["userInfo"];
    let subject = text(&user["userId"]);
    if !subject.is_empty() {
        q.identity = Some(AccountIdentity {
            key: format!("kiro-user:{}", hash(subject)),
            email: label(&user["email"]).unwrap_or_default(),
            checked_at: now(),
            ..Default::default()
        });
    }
    Ok(())
}

fn amount(s: &str) -> Option<f64> {
    let n: f64 = s.trim().replace(',', "").parse().ok()?;
    (n.is_finite() && n >= 0.).then_some(n)
}
fn pair(s: &str, separator: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(separator)?;
    Some((amount(a)?, amount(b)?))
}
/// `Welcome bonus: 40.25/500` or, from older releases, a bare `0.00/100`.
fn bonus_parts(s: &str) -> Option<(&str, (f64, f64))> {
    let (name, numbers) = s.rsplit_once(':').map_or(("", s), |(n, x)| (n.trim(), x));
    Some((name, pair(numbers, "/")?))
}
/// `YYYY-MM-DD` (UTC) or, from older releases, `MM/DD` in the coming year.
fn cli_date(s: &str) -> Option<i64> {
    let token: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '/'))
        .collect();
    let date = if let Ok(date) = chrono::NaiveDate::parse_from_str(&token, "%Y-%m-%d") {
        date
    } else {
        let (month, day) = token.split_once('/')?;
        let (month, day) = (month.parse().ok()?, day.parse().ok()?);
        let today = chrono::Utc::now().date_naive();
        let date = chrono::NaiveDate::from_ymd_opt(today.year(), month, day)?;
        if date < today {
            chrono::NaiveDate::from_ymd_opt(today.year() + 1, month, day)?
        } else {
            date
        }
    };
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

/// Text printed by `kiro-cli chat --no-interactive /usage`, without colours.
/// kiro-cli 2.36.0 prints, for example:
///
/// ```text
/// Estimated Usage | resets on 2026-11-19 | KIRO PRO
/// Bonus credits
/// Welcome bonus: 40.25/500 credits used, expires in 5 days
/// Credits (812.37 of 1000 covered in plan), 81.2%
/// ```
///
/// Older releases print `Plan: …`, `| KIRO FREE`, `Bonus credits: x/y credits
/// used` and `resets on MM/DD`. Add-on packs read `x of y credits across n
/// packs`; overage reads `Overages credits: x credits`.
pub(crate) fn parse_cli(source: &Source, raw: &str) -> Result<QuotaSnapshot> {
    let plain = strip_ansi(raw);
    let lower = plain.to_ascii_lowercase();
    ensure!(
        !lower.contains("not logged in") && !lower.contains("kiro-cli login"),
        RELOGIN
    );
    let mut q = QuotaSnapshot {
        source_id: source.id.clone(),
        account_id: source.account_id.clone(),
        provider: source.provider.clone(),
        name: source.name.clone(),
        updated_at: now(),
        origin: "command".into(),
        ..Default::default()
    };
    let reset = plain
        .find("resets on ")
        .and_then(|i| cli_date(&plain[i + "resets on ".len()..]));
    let meter = |id: String, label: String, used: f64, total: Option<f64>, resets_at| QuotaMeter {
        id,
        label,
        used,
        total,
        remaining: total.map(|t| (t - used).max(0.)),
        unit: "credits".into(),
        resets_at,
        ..Default::default()
    };
    let mut bonus = 0;
    for line in plain.lines().map(str::trim) {
        if let Some(end) = line.find(" covered in plan")
            && let Some((used, total)) = line[..end]
                .rsplit_once('(')
                .and_then(|(_, s)| pair(s, " of "))
        {
            if !q.meters.iter().any(|m| m.id == "plan") {
                q.meters.push(meter(
                    "plan".into(),
                    "套餐 Credits".into(),
                    used,
                    Some(total),
                    reset,
                ));
            }
        } else if let Some(end) = line.find(" credits used")
            && let Some((name, (used, total))) = bonus_parts(&line[..end])
        {
            let expires = line
                .find("expires in ")
                .and_then(|i| amount(line[i + "expires in ".len()..].split_whitespace().next()?))
                .map(|days| now() + (days * 86400.) as i64);
            let label = if name.is_empty() || name.eq_ignore_ascii_case("bonus credits") {
                "赠送 Credits".into()
            } else {
                name.chars().take(64).collect()
            };
            let mut m = meter(format!("bonus-{bonus}"), label, used, Some(total), expires);
            m.group = "赠送".into();
            q.meters.push(m);
            bonus += 1;
        } else if let Some(end) = line.find(" credits across ")
            && let Some((used, total)) = pair(&line[..end], " of ")
        {
            q.meters.push(meter(
                "add-on".into(),
                "加购 Credits".into(),
                used,
                Some(total),
                None,
            ));
        } else if let Some(rest) = line.strip_prefix("Overages credits:")
            && let Some(used) = amount(rest.trim().trim_end_matches("credits"))
        {
            q.meters.push(meter(
                "overage".into(),
                "超额 Credits".into(),
                used,
                None,
                reset,
            ));
        }
    }
    q.plan = plain
        .lines()
        .map(str::trim)
        .find_map(|line| {
            let name = if line.starts_with("Estimated Usage") {
                line.rsplit('|').next()?
            } else if let Some(rest) = line.strip_prefix("Plan:") {
                rest.split('|').next()?
            } else {
                line.strip_prefix('|')
                    .filter(|r| r.trim_start().starts_with("KIRO"))?
            };
            let name = name.trim();
            (!name.is_empty() && name.len() <= 64 && !name.starts_with("resets on"))
                .then(|| name.to_owned())
        })
        .or_else(|| Some("Kiro".into()));
    if q.meters.is_empty() {
        // Organization plans can withhold the numbers.
        ensure!(
            lower.contains("managed by admin") || lower.contains("managed by organization"),
            "Kiro 用量输出格式不支持"
        );
        q.notice = Some("Kiro 套餐由组织管理员管理，额度请在 Kiro 控制台查看".into());
    } else {
        q.notice = Some("登录已过期，额度由 kiro-cli 查询".into());
    }
    Ok(q)
}

/// Removes CSI (`ESC [ … final`) and OSC (`ESC ] … BEL` or `ESC \`) sequences.
pub(crate) fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c == '\r' {
            out.push('\n');
        } else if !c.is_control() || c == '\n' || c == '\t' {
            out.push(c);
        }
    }
    out
}

/// `--v2` keeps `/usage` local: kiro-cli's V3 engine sends a non-interactive
/// `/usage` to the model as a prompt (checked with kiro-cli 2.36.0).
pub(crate) const CLI_ARGS: &[&str] = &["chat", "--v2", "--no-interactive", "/usage"];

/// `kiro-cli chat --v2 --no-interactive /usage` in an empty directory Aieyes
/// owns, marked `AIEYES_INTERNAL` and without kiro-cli's self-update.
pub(crate) fn cli_command(
    program: &str,
    dir: &std::path::Path,
    settings: &Settings,
) -> std::process::Command {
    let mut command = crate::process::cli_command_with_pre_command(
        program,
        CLI_ARGS,
        if cfg!(unix) { ":" } else { "" },
        "",
    );
    command.current_dir(dir);
    network::apply_env(&mut command, &settings.proxy);
    command
        .env("AIEYES_INTERNAL", "1")
        .env("KIRO_NO_AUTO_UPDATE", "1")
        .env("NO_COLOR", "1");
    command
}

/// Runs the check for at most 20 s. Stdin is closed at once: with an open pipe
/// kiro-cli waits to read more of the prompt.
pub(crate) fn run_cli(program: &str, dir: &std::path::Path, settings: &Settings) -> Result<String> {
    let output = crate::process::run(
        cli_command(program, dir, settings),
        Vec::new(),
        std::time::Duration::from_secs(20),
    );
    match output {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.to_string().contains("（127）") || e.to_string().contains("启动命令失败") =>
        {
            bail!("未找到 kiro-cli；{RELOGIN}")
        }
        // A timeout or a cancelled run says nothing about the login.
        Err(e) if e.to_string() == "查询超时" || crate::operations::check().is_err() => Err(e),
        Err(_) => bail!("{RELOGIN}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field set of `GetUsageLimits` as consumed by Kiro IDE 1.4.62 and the
    /// kiro-cli 2.36.0 `/usage` view. Values are invented.
    fn response(future: i64) -> Value {
        json!({
            "nextDateReset": 1_795_104_000.0,
            "overageConfiguration": {"overageStatus":"ENABLED"},
            "subscriptionInfo": {"subscriptionTitle":"KIRO PRO+","type":"Q_DEVELOPER_STANDALONE_PRO_PLUS"},
            "userInfo": {"userId":"u-1","email":"dev@example.com"},
            "usageBreakdownList": [
                {"resourceType":"AGENTIC_REQUEST","currentUsage":1,"usageLimit":2},
                {"resourceType":"CREDIT","currency":"USD","displayNamePlural":"Credits",
                "currentUsage":2012,"currentUsageWithPrecision":2012.5,"usageLimit":2000,"usageLimitWithPrecision":2000.0,
                "currentOverages":12,"currentOveragesWithPrecision":12.5,"overageCapWithPrecision":500.0,"overageRate":0.04,"overageCharges":0.5,
                "nextDateReset":1_795_104_000.0,
                "bonuses":[
                    {"bonusCode":"WELCOME","displayName":"Welcome bonus","status":"ACTIVE","expiresAt":future,"currentUsage":5,"usageLimit":20},
                    {"bonusCode":"OLD","status":"EXPIRED","expiresAt":future,"currentUsage":0,"usageLimit":10},
                    {"bonusCode":"USED","status":"EXHAUSTED","expiresAt":future,"currentUsage":10,"usageLimit":10},
                    {"bonusCode":"GONE","currentUsage":0,"usageLimit":10,"expiresAt":1}],
                "freeTrialInfo":{"freeTrialStatus":"ACTIVE","currentUsageWithPrecision":10,"usageLimitWithPrecision":500,"freeTrialExpiry":future},
                "overageCredits":[
                    {"currentUsage":3,"usageLimit":100,"expiresAt":future},
                    {"currentUsage":0,"usageLimit":50,"expiresAt":future + 10},
                    {"currentUsage":0,"usageLimit":50,"expiresAt":1}]}
            ]
        })
    }

    fn cli(raw: &str) -> Result<QuotaSnapshot> {
        parse_cli(&Source::default(), raw)
    }

    #[test]
    fn get_usage_limits_becomes_plan_overage_bonus_and_add_on_meters() {
        let future = now() + 86400;
        let mut q = QuotaSnapshot::default();
        parse(&mut q, &response(future)).unwrap();
        let ids: Vec<_> = q.meters.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            ["plan", "overage", "bonus-0", "bonus-2", "trial", "add-on"]
        );
        // `currentUsage` includes overage; the plan shows its own part.
        let plan = &q.meters[0];
        assert_eq!(
            (plan.used, plan.total, plan.remaining),
            (2000., Some(2000.), Some(0.))
        );
        assert_eq!(plan.resets_at, Some(1_795_104_000));
        assert_eq!(q.meters[1].used, 12.5);
        assert_eq!(q.meters[1].total, Some(500.));
        assert_eq!(q.meters[2].label, "Welcome bonus");
        assert_eq!(q.meters[3].label, "USED");
        assert_eq!(q.meters[4].label, "试用 Credits");
        assert_eq!(q.meters[4].remaining, Some(490.));
        assert_eq!((q.meters[5].used, q.meters[5].total), (3., Some(150.)));
        assert_eq!(q.meters[5].resets_at, Some(future));
        assert!(q.meters.iter().all(QuotaMeter::valid));
        assert_eq!(q.plan.as_deref(), Some("KIRO PRO+"));
        assert!(q.identity.unwrap().key.starts_with("kiro-user:"));

        // Overage disabled, trial over, unlimited plan.
        let mut v = response(future);
        v["overageConfiguration"]["overageStatus"] = json!("DISABLED");
        let credit = &mut v["usageBreakdownList"][1];
        credit["currentOverages"] = json!(0);
        credit["currentOveragesWithPrecision"] = json!(0);
        credit["usageLimitWithPrecision"] = json!(999_999);
        credit["freeTrialInfo"]["freeTrialStatus"] = json!("EXPIRED");
        let mut q = QuotaSnapshot::default();
        parse(&mut q, &v).unwrap();
        assert!(
            !q.meters
                .iter()
                .any(|m| m.id == "overage" || m.id == "trial")
        );
        assert_eq!((q.meters[0].used, q.meters[0].total), (2012.5, None));

        // Top-level shape, no overage.
        let mut flat = QuotaSnapshot::default();
        parse(
            &mut flat,
            &json!({"currentUsage":10,"usageLimit":50,"nextDateReset":"2026-12-01T00:00:00Z"}),
        )
        .unwrap();
        assert_eq!(flat.meters.len(), 1);
        assert_eq!(flat.meters[0].remaining, Some(40.));
        assert_eq!(flat.plan.as_deref(), Some("Kiro"));
        assert!(parse(&mut QuotaSnapshot::default(), &json!({"x":1})).is_err());
    }

    #[test]
    fn region_follows_the_profile_arn_and_unknown_regions_are_refused() {
        assert!(
            region_url("arn:aws:codewhisperer:us-east-1:1:profile/A")
                .unwrap()
                .contains("us-east-1")
        );
        assert_eq!(
            region_url("arn:aws:codewhisperer:eu-central-1:1:profile/A").unwrap(),
            "https://q.eu-central-1.amazonaws.com/"
        );
        assert!(region_url("arn:aws:codewhisperer:ap-south-9:1:p").is_err());
        assert!(region_url("arn:aws:s3:us-east-1:1:p").is_err());
    }

    #[test]
    fn usage_text_of_kiro_cli_2_36_and_older_releases_is_parsed() {
        // Captured from kiro-cli 2.36.0 against a local GetUsageLimits fixture.
        let current = "Estimated Usage | resets on 2026-11-19 | KIRO PRO+\nBonus credits\nWelcome bonus: 40.25/500 credits used, expires in 5 days\nCredits (2012.50 of 2000 covered in plan), 100.6%\nAdditional credits:\n  Add-on credits available for purchase\nManage your plan or purchase add-on credits at https://app.kiro.dev/account/usage\n";
        let q = cli(current).unwrap();
        assert_eq!(q.plan.as_deref(), Some("KIRO PRO+"));
        let plan = q.meters.iter().find(|m| m.id == "plan").unwrap();
        assert_eq!(
            (plan.used, plan.total, plan.remaining),
            (2012.5, Some(2000.), Some(0.))
        );
        let reset = chrono::NaiveDate::from_ymd_opt(2026, 11, 19)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(plan.resets_at, Some(reset));
        let bonus = q.meters.iter().find(|m| m.id == "bonus-0").unwrap();
        assert_eq!(
            (bonus.label.as_str(), bonus.used, bonus.total),
            ("Welcome bonus", 40.25, Some(500.))
        );
        assert!(bonus.resets_at.unwrap() > now() + 4 * 86400);
        assert_eq!(
            q.notice.as_deref(),
            Some("登录已过期，额度由 kiro-cli 查询")
        );
        assert!(q.meters.iter().all(QuotaMeter::valid));

        let colored = "\u{1b}]0;kiro\u{7}\u{1b}[1mEstimated Usage\u{1b}[0m | resets on 2026-11-19 | KIRO FREE\r\nCredits (1,204.25 of 2,000 covered in plan), 60.2%\n    12.00 of 100 credits across 2 packs\nOverages credits: 3.5 credits\n";
        let q = cli(colored).unwrap();
        assert_eq!(q.plan.as_deref(), Some("KIRO FREE"));
        let ids: Vec<_> = q.meters.iter().map(|m| (m.id.as_str(), m.used)).collect();
        assert_eq!(ids, [("plan", 1204.25), ("add-on", 12.), ("overage", 3.5)]);
        assert_eq!(q.meters[0].remaining, Some(795.75));

        // Pre-2.x layout (CodexBar's documented sample).
        let legacy = "| KIRO FREE\nMonthly credits:\n████ 0% (resets on 01/01)\n(0.00 of 50 covered in plan)\nBonus credits:\n0.00/100 credits used, expires in 88 days\n";
        let q = cli(legacy).unwrap();
        assert_eq!(q.plan.as_deref(), Some("KIRO FREE"));
        assert_eq!(q.meters[0].total, Some(50.));
        assert!(q.meters[0].resets_at.unwrap() > now() - 86400);
        assert_eq!(q.meters[1].label, "赠送 Credits");
        let q = cli("Plan: Q Developer Pro | 1 usage breakdowns\nYour plan is managed by admin\n")
            .unwrap();
        assert!(q.meters.is_empty() && q.notice.is_some());
        assert_eq!(q.plan.as_deref(), Some("Q Developer Pro"));

        assert_eq!(
            cli("Not logged in. Set the KIRO_API_KEY environment variable or run `kiro-cli login` first.")
                .unwrap_err()
                .to_string(),
            RELOGIN
        );
        assert!(cli("Hello! How can I help you today?").is_err());
    }

    #[test]
    fn cli_fallback_is_internal_local_and_never_updates_kiro_cli() {
        let dir = tempfile::tempdir().unwrap();
        let command = cli_command("kiro-cli", dir.path(), &Settings::default());
        let envs: Vec<_> = command
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v?.to_str()?.to_owned())))
            .collect();
        assert!(envs.contains(&("AIEYES_INTERNAL".into(), "1".into())));
        assert!(envs.contains(&("KIRO_NO_AUTO_UPDATE".into(), "1".into())));
        assert_eq!(command.get_current_dir(), Some(dir.path()));
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy()).collect();
        assert!(args.join(" ").contains("chat") && args.join(" ").contains("--v2"));
        assert_eq!(
            crate::session_events::headless_argv(
                "kiro",
                &std::iter::once("kiro-cli")
                    .chain(CLI_ARGS.iter().copied())
                    .map(String::from)
                    .collect::<Vec<_>>()
            ),
            Some(crate::session_events::HEADLESS)
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_cli_runs_with_stdin_closed_and_errors_never_quote_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("kiro-cli");
        // `cat` returns at once only when stdin is closed.
        std::fs::write(
            &program,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' \"$*\" > args\nprintf '%s %s\\n' \"$AIEYES_INTERNAL\" \"$KIRO_NO_AUTO_UPDATE\" > env\nprintf 'Estimated Usage | resets on 2026-11-19 | KIRO PRO\\nCredits (10.00 of 50 covered in plan), 20.0%%\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let out = run_cli(program.to_str().unwrap(), dir.path(), &Settings::default()).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(cli(&out).unwrap().meters[0].used, 10.);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("args")).unwrap(),
            "chat --v2 --no-interactive /usage\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("env")).unwrap(),
            "1 1\n"
        );

        std::fs::write(&program, "#!/bin/sh\necho 'PRIVATE detail' >&2\nexit 1\n").unwrap();
        let error =
            run_cli(program.to_str().unwrap(), dir.path(), &Settings::default()).unwrap_err();
        assert_eq!(error.to_string(), RELOGIN);
        // Resolved through PATH like the real `kiro-cli`.
        let error =
            run_cli("kiro-cli-aieyes-missing", dir.path(), &Settings::default()).unwrap_err();
        assert!(error.to_string().starts_with("未找到 kiro-cli"));
    }
}
