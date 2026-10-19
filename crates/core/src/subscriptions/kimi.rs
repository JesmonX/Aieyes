//! Official kimi-code/oauth managed-usage protocol. Wallet fixed points are
//! 1,000,000 per cent; monthly money fields are ordinary priceInCents.
use super::*;

fn integer(value: &Value) -> Result<i128> {
    if let Some(n) = value.as_i64() {
        return Ok(n.into());
    }
    if let Some(n) = value.as_u64() {
        return Ok(n.into());
    }
    if let Some(s) = value.as_str() {
        let digits = s.strip_prefix('-').unwrap_or(s);
        ensure!(
            !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()),
            "Kimi 钱包金额格式无效"
        );
        return s
            .parse()
            .map_err(|_| anyhow::anyhow!("Kimi 钱包金额超出支持范围"));
    }
    if let Some(n) = value.as_f64() {
        ensure!(
            n.is_finite() && n.fract() == 0. && n.abs() <= 9_007_199_254_740_991.,
            "Kimi 钱包金额不是可精确表示的整数"
        );
        return Ok(n as i128);
    }
    bail!("Kimi 钱包金额未提供")
}

fn decimal(value: i128, scale: u32) -> String {
    let magnitude = value.unsigned_abs();
    let unit = 10u128.pow(scale);
    let mut fraction = format!("{:0width$}", magnitude % unit, width = scale as usize);
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!(
        "{}{}.{}",
        if value < 0 { "-" } else { "" },
        magnitude / unit,
        fraction
    )
}

fn wallet(raw: &Value) -> Result<(Option<ExtraUsageWallet>, bool)> {
    if raw.is_null() {
        return Ok((None, false));
    }
    ensure!(
        raw.is_object() && raw["balance"]["type"] == "BOOSTER",
        "Kimi 钱包类型不支持"
    );
    let total = integer(&raw["balance"]["amount"])?;
    ensure!(total >= 0, "Kimi 钱包总额无效");
    if total == 0 {
        ensure!(
            raw["balance"]["amountLeft"].is_null() || integer(&raw["balance"]["amountLeft"])? == 0,
            "Kimi 钱包金额不一致"
        );
        return Ok((None, false));
    }
    let mut currencies = Vec::new();
    for name in ["monthlyChargeLimit", "monthlyUsed"] {
        let money = &raw[name];
        if !money.is_null() {
            ensure!(money.is_object(), "Kimi 钱包月度金额格式无效");
            if !money["currency"].is_null() {
                let currency = money["currency"].as_str().context("Kimi 钱包币种无效")?;
                if !currency.is_empty() {
                    ensure!(
                        currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_alphabetic()),
                        "Kimi 钱包币种格式不支持"
                    );
                    currencies.push(currency.to_ascii_uppercase());
                }
            }
        }
    }
    let currency = currencies.first().map(String::as_str).unwrap_or("USD");
    ensure!(
        currencies.iter().all(|c| c.as_str() == currency),
        "Kimi 钱包币种不一致"
    );
    let mut partial = false;
    let mut amount = |value: &Value, scale: u32, nonnegative: bool| {
        if value.is_null() {
            return None;
        }
        match integer(value) {
            Ok(n) if !nonnegative || n >= 0 => Some(decimal(n, scale)),
            _ => {
                partial = true;
                None
            }
        }
    };
    let balance = amount(&raw["balance"]["amountLeft"], 8, false);
    let monthly_limit = amount(&raw["monthlyChargeLimit"]["priceInCents"], 2, true);
    let monthly_used = amount(&raw["monthlyUsed"]["priceInCents"], 2, false);
    let monthly_limit_enabled = raw["monthlyChargeLimitEnabled"].as_bool();
    partial |= balance.is_none()
        || !raw["monthlyChargeLimitEnabled"].is_null() && monthly_limit_enabled.is_none()
        || monthly_limit_enabled == Some(true)
            && (monthly_limit.is_none() || monthly_used.is_none());
    Ok((
        Some(ExtraUsageWallet {
            currency: currency.into(),
            total: decimal(total, 8),
            balance,
            monthly_limit_enabled,
            monthly_limit,
            monthly_used,
        }),
        partial,
    ))
}

pub(super) fn apply(q: &mut QuotaSnapshot, value: &Value) -> Result<()> {
    let mut partial = false;
    let usages = &value["usages"];
    if !usages.is_null() && !usages.is_object() {
        partial = true;
    }
    for (key, name, minutes) in [
        ("limit_5h", "5h", Some(300)),
        ("limit_7d", "7d", Some(10080)),
        ("limit_month_total", "月总额度", None),
        ("limit_month_code", "月编程额度", None),
    ] {
        let meter = &usages[key];
        if meter.is_null() {
            continue;
        }
        let parsed = (|| -> Result<(f64, Option<i64>)> {
            let used = number(&meter["used_ratio"])?;
            ensure!(used >= 0., "Kimi Code 额度比例无效");
            let reset = if meter["reset_time"].is_null() {
                None
            } else {
                Some(stamp(&meter["reset_time"]).context("Kimi Code 重置时间无效")?)
            };
            Ok((used, reset))
        })();
        match parsed {
            Ok((used, reset)) => window(q, key, name, (used * 100.).min(100.), minutes, reset),
            Err(_) => partial = true,
        }
    }
    match wallet(&value["boosterWallet"]) {
        Ok((wallet, incomplete)) => {
            q.extra_usage = wallet;
            partial |= incomplete;
        }
        Err(_) => partial = true,
    }
    if partial {
        q.metadata_error = Some("Kimi 部分额度信息未返回有效数据；已保留有效结果".into());
    }
    Ok(())
}
