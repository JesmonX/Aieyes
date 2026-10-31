//! Prepaid wallets of Cline, Kilo Code and OpenRouter keys found in their
//! clients' own files. Tokens are never refreshed: Cline guards refresh with its
//! own lock and Kilo tokens are long-lived; an expired one is a re-login notice.
use super::*;

/// Organisations queried per login; the rest are reported, not fetched.
const MAX_ORGS: usize = 8;
pub(super) const CLINE_API: &str = "https://api.cline.bot/api/v1";
pub(super) const KILO_API: &str = "https://api.kilo.ai/api";
pub(super) const OPENROUTER_KEY: &str = "https://openrouter.ai/api/v1/key";

/// Path segment taken from a server response; anything unusual is refused
/// instead of being escaped into another route.
fn segment(id: &str) -> Result<&str> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':'))
            && !id.starts_with('.'),
        "账户标识格式不支持"
    );
    Ok(id)
}

/// Cline wraps every response as `{success, data}`.
fn unwrap(v: Value) -> Result<Value> {
    if let Some(ok) = v["success"].as_bool() {
        ensure!(ok, "Cline 账户查询被拒绝");
        return Ok(v["data"].clone());
    }
    Ok(v)
}

fn cline_token(secret: &str) -> String {
    if secret.to_ascii_lowercase().starts_with("workos:") {
        secret.to_owned()
    } else {
        format!("workos:{secret}")
    }
}

fn kilo_user(profile: &Value) -> Value {
    json!({"id":profile["user"]["id"],
        "email":profile["user"]["email"].as_str().or(profile["email"].as_str())})
}

fn identity(provider: &str, me: &Value) -> Option<AccountIdentity> {
    let subject = text(&me["id"]);
    let email = label(&me["email"]).unwrap_or_default();
    let key = if subject.is_empty() {
        email.to_ascii_lowercase()
    } else {
        subject.to_owned()
    };
    (!key.is_empty()).then(|| AccountIdentity {
        key: format!("{provider}:{}", hash(&key)),
        email,
        checked_at: now(),
        ..Default::default()
    })
}

/// Resolve opaque native tokens with a single profile read. The same identity
/// function labels wallet responses, so a switched login cannot reuse a balance.
pub(super) fn inspect_identity(
    provider: &str,
    settings: &Settings,
    secret: &str,
) -> Result<AccountIdentity> {
    let source = Source {
        provider: provider.into(),
        ..Default::default()
    };
    let config = SubscriptionConfig::default();
    inspect_identity_using(provider, secret, |token, url| {
        request(&source, settings, &config, token, url)
    })
}

pub(super) fn inspect_identity_using(
    provider: &str,
    secret: &str,
    mut read: impl FnMut(&str, &str) -> Result<Value>,
) -> Result<AccountIdentity> {
    let me = match provider {
        "cline" => unwrap(read(
            &cline_token(secret),
            &format!("{CLINE_API}/users/me"),
        )?)?,
        "kilo" => kilo_user(&read(secret, &format!("{KILO_API}/profile"))?),
        _ => bail!("不支持的账户类型"),
    };
    identity(provider, &me).context("账户响应缺少身份")
}

/// Cline: `/users/me`, the personal balance and each organisation's balance,
/// combined into one document so that `parse` stays a pure function.
pub(super) fn cline_using(
    secret: &str,
    url: &str,
    mut read: impl FnMut(&str, &str) -> Result<Value>,
) -> Result<Value> {
    // Stored WorkOS tokens carry their prefix; older files may not.
    let token = cline_token(secret);
    let me = unwrap(read(&token, url)?)?;
    let user = segment(text(&me["id"]))?.to_owned();
    let balance = unwrap(read(&token, &format!("{CLINE_API}/users/{user}/balance"))?)?;
    let mut organizations = vec![];
    for org in me["organizations"].as_array().into_iter().flatten() {
        let id = text(&org["organizationId"]);
        let mut entry = json!({"id":id,"name":org["name"],"active":org["active"]});
        if organizations.len() >= MAX_ORGS {
            entry["skipped"] = json!(true);
        } else {
            match segment(id).and_then(|id| {
                read(&token, &format!("{CLINE_API}/organizations/{id}/balance")).and_then(unwrap)
            }) {
                Ok(v) => entry["balance"] = v["balance"].clone(),
                Err(_) => entry["error"] = json!(true),
            }
        }
        organizations.push(entry);
    }
    Ok(
        json!({"me":{"id":me["id"],"email":me["email"]},"balance":balance["balance"],"organizations":organizations}),
    )
}

/// Kilo: `/profile`, the personal balance unless there is no personal account,
/// and each organisation's balance through `x-kilocode-organizationid`.
pub(super) fn kilo_using(
    url: &str,
    mut read: impl FnMut(&str, &[(&str, &str)]) -> Result<Value>,
) -> Result<Value> {
    let profile = read(url, &[])?;
    let balance_url = format!("{KILO_API}/profile/balance");
    let personal = if profile["hasPersonalAccount"] == false {
        Value::Null
    } else {
        read(&balance_url, &[])?["balance"].clone()
    };
    let mut organizations = vec![];
    for org in profile["organizations"].as_array().into_iter().flatten() {
        let id = text(&org["id"]);
        let mut entry = json!({"id":id,"name":org["name"]});
        if organizations.len() >= MAX_ORGS {
            entry["skipped"] = json!(true);
        } else {
            match segment(id)
                .and_then(|id| read(&balance_url, &[("x-kilocode-organizationid", id)]))
            {
                Ok(v) => entry["balance"] = v["balance"].clone(),
                Err(_) => entry["error"] = json!(true),
            }
        }
        organizations.push(entry);
    }
    Ok(json!({
        "me":kilo_user(&profile),
        "balance":personal,
        "organizations":organizations
    }))
}

fn amount(v: &Value) -> Result<f64> {
    let n = number(v)?;
    ensure!(n >= 0., "余额数值无效");
    Ok(n)
}

fn wallet(q: &mut QuotaSnapshot, id: String, group: &str, label: String, left: f64, unit: &str) {
    q.meters.push(QuotaMeter {
        id,
        group: group.into(),
        label,
        used: 0.,
        remaining: Some(left),
        unit: unit.into(),
        ..Default::default()
    });
}

/// Combined document from `cline` / `kilo`: one paid wallet per owner.
pub(super) fn wallets(q: &mut QuotaSnapshot, provider: &str, v: &Value) -> Result<()> {
    let (unit, personal) = if provider == "cline" {
        // Cline reports account Credits; the unit is not documented as USD.
        ("credits", "个人 Credits")
    } else {
        ("usd", "个人余额")
    };
    if !v["balance"].is_null() {
        wallet(
            q,
            "personal".into(),
            "",
            personal.into(),
            amount(&v["balance"])?,
            unit,
        );
    }
    let mut missing = 0;
    for org in v["organizations"].as_array().into_iter().flatten() {
        if org["skipped"] == true || org["error"] == true || org["balance"].is_null() {
            missing += 1;
            continue;
        }
        let name = label(&org["name"]).unwrap_or_else(|| "组织".into());
        wallet(
            q,
            format!("org-{}", hash(text(&org["id"]))),
            "组织",
            name,
            amount(&org["balance"])?,
            unit,
        );
    }
    if missing > 0 {
        q.metadata_error = Some(format!("{missing} 个组织的余额暂不可用"));
    }
    q.identity = identity(provider, &v["me"]);
    q.plan = Some(if provider == "cline" {
        "Cline 预付 Credits".into()
    } else {
        "Kilo 预付余额".into()
    });
    if q.meters.is_empty() {
        q.notice = Some("账户未返回余额".into());
    }
    Ok(())
}

/// OpenRouter `GET /api/v1/key`: the key's own limit and what is left of it.
pub(super) fn openrouter(q: &mut QuotaSnapshot, v: &Value) -> Result<()> {
    let d = &v["data"];
    ensure!(d.is_object(), "OpenRouter 响应格式不支持");
    let limit = (!d["limit"].is_null())
        .then(|| amount(&d["limit"]))
        .transpose()?;
    let reported_remaining = (!d["limit_remaining"].is_null())
        .then(|| amount(&d["limit_remaining"]))
        .transpose()?;
    let reset = text(&d["limit_reset"]);
    let (usage_field, byok_field) = match reset {
        "daily" => ("usage_daily", "byok_usage_daily"),
        "weekly" => ("usage_weekly", "byok_usage_weekly"),
        "monthly" => ("usage_monthly", "byok_usage_monthly"),
        "" => ("usage", "byok_usage"),
        _ => bail!("OpenRouter 响应格式不支持"),
    };
    // `usage` is lifetime usage, never the spend for a resettable key limit.
    // BYOK only consumes that limit when the service explicitly includes it.
    let includes_byok = d["include_byok_in_limit"] == true;
    let used = if !d[usage_field].is_null() && (!includes_byok || !d[byok_field].is_null()) {
        amount(&d[usage_field])?
            + if includes_byok {
                amount(&d[byok_field])?
            } else {
                0.
            }
    } else if let (Some(limit), Some(remaining)) = (limit, reported_remaining) {
        (limit - remaining).max(0.)
    } else {
        bail!("OpenRouter 响应格式不支持");
    };
    let remaining = reported_remaining.or_else(|| limit.map(|limit| (limit - used).max(0.)));
    q.meters.push(QuotaMeter {
        id: "openrouter-key".into(),
        label: match reset {
            "daily" => "OpenRouter Key · 每日".into(),
            "weekly" => "OpenRouter Key · 每周".into(),
            "monthly" => "OpenRouter Key · 每月".into(),
            _ => "OpenRouter Key".into(),
        },
        used,
        total: limit,
        remaining,
        unit: "usd".into(),
        ..Default::default()
    });
    q.plan = Some(if d["is_free_tier"] == true {
        "OpenRouter 免费额度".into()
    } else {
        "OpenRouter".into()
    });
    if limit.is_none() {
        q.notice = Some("此 Key 未设置限额；账户余额请在 OpenRouter 查看".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> QuotaSnapshot {
        QuotaSnapshot::default()
    }

    #[test]
    fn native_profile_reads_one_endpoint_and_matches_wallet_owner() {
        for (provider, reply, expected) in [
            (
                "cline",
                json!({"success":true,"data":{"id":"cline-user","email":"User@Example.test"}}),
                "cline-user",
            ),
            (
                "kilo",
                json!({"user":{"id":"kilo-user","email":"User@Example.test"},"organizations":[{"id":"different-org"}]}),
                "kilo-user",
            ),
            (
                "kilo",
                json!({"user":{"email":"User@Example.test"},"organizations":[{"id":"different-org"}]}),
                "user@example.test",
            ),
        ] {
            let mut calls = 0;
            let owner = inspect_identity_using(provider, "fixture-token", |token, url| {
                calls += 1;
                assert_eq!(
                    token,
                    if provider == "cline" {
                        "workos:fixture-token"
                    } else {
                        "fixture-token"
                    }
                );
                assert_eq!(
                    url,
                    if provider == "cline" {
                        format!("{CLINE_API}/users/me")
                    } else {
                        format!("{KILO_API}/profile")
                    }
                );
                Ok(reply.clone())
            })
            .unwrap();
            assert_eq!(
                calls, 1,
                "discovery must not read wallets or refresh tokens"
            );
            assert_eq!(owner.key, format!("{provider}:{}", hash(expected)));
            let me = if provider == "cline" {
                reply["data"].clone()
            } else {
                reply["user"].clone()
            };
            let mut quota = snapshot();
            wallets(&mut quota, provider, &json!({"me":me,"balance":0})).unwrap();
            assert_eq!(quota.identity.unwrap().key, owner.key);
        }
        assert!(inspect_identity_using("kilo", "fixture-token", |_, _| {
            Ok(json!({"accountId":"organization-only","organizations":[{"id":"organization-only"}]}))
        }).is_err(), "an organization is not the signed-in user");
        assert!(
            inspect_identity_using("cline", "fixture-token", |_, _| {
                Ok(json!({"success":false,"data":{"id":"untrusted"}}))
            })
            .is_err()
        );
    }

    #[test]
    fn cline_request_chain_uses_native_dto_and_keeps_successful_org_wallets() {
        let mut urls = vec![];
        let body = cline_using("fixture", &format!("{CLINE_API}/users/me"), |token, url| {
            assert_eq!(token, "workos:fixture");
            urls.push(url.to_owned());
            let data = match url.strip_prefix(CLINE_API).unwrap() {
                "/users/me" => json!({"id":"usr_1","email":"a@example.test","organizations":[
                    {"organizationId":"org_1","name":"Unavailable","active":true},
                    {"organizationId":"org_2","name":"Team","active":true}
                ]}),
                "/users/usr_1/balance" => json!({"userId":"usr_1","balance":12.5}),
                "/organizations/org_1/balance" => bail!("fixture failure"),
                "/organizations/org_2/balance" => json!({"organizationId":"org_2","balance":40}),
                other => panic!("unexpected route {other}"),
            };
            Ok(json!({"success":true,"data":data}))
        })
        .unwrap();
        assert_eq!(urls.len(), 4);
        let mut quota = snapshot();
        wallets(&mut quota, "cline", &body).unwrap();
        assert_eq!(quota.meters.len(), 2);
        assert_eq!(quota.meters[1].remaining, Some(40.));
        assert_eq!(
            quota.metadata_error.as_deref(),
            Some("1 个组织的余额暂不可用")
        );
    }

    #[test]
    fn kilo_raw_profile_uses_org_headers_without_querying_absent_personal_account() {
        let mut calls = 0;
        let body = kilo_using(&format!("{KILO_API}/profile"), |url, headers| {
            calls += 1;
            if url.ends_with("/profile") {
                assert!(headers.is_empty());
                return Ok(json!({"user":{"email":"B@Example.test","name":"Fixture"},"hasPersonalAccount":false,
                    "organizations":[{"id":"org_1","name":"Team"}]}));
            }
            assert_eq!(url, format!("{KILO_API}/profile/balance"));
            assert_eq!(headers, &[("x-kilocode-organizationid", "org_1")]);
            Ok(json!({"balance":0}))
        }).unwrap();
        assert_eq!(calls, 2);
        let mut quota = snapshot();
        wallets(&mut quota, "kilo", &body).unwrap();
        assert_eq!(quota.meters.len(), 1);
        assert_eq!(quota.meters[0].remaining, Some(0.));
        assert_eq!(
            quota.identity.unwrap().key,
            format!("kilo:{}", hash("b@example.test"))
        );
    }

    #[test]
    fn cline_wallets_keep_personal_and_each_organisation() {
        let mut q = snapshot();
        let v = json!({"me":{"id":"usr_1","email":"a@example.com"},"balance":12.5,"organizations":[
            {"id":"org_1","name":"Team","balance":"40"},
            {"id":"org_2","name":"Other","error":true}
        ]});
        wallets(&mut q, "cline", &v).unwrap();
        assert_eq!(q.meters.len(), 2);
        assert_eq!(q.meters[0].remaining, Some(12.5));
        assert_eq!(q.meters[0].unit, "credits");
        assert_eq!(q.meters[1].label, "Team");
        assert_eq!(q.meters[1].group, "组织");
        assert!(q.meters.iter().all(QuotaMeter::valid));
        assert_eq!(q.metadata_error.as_deref(), Some("1 个组织的余额暂不可用"));
        let identity = q.identity.unwrap();
        assert_eq!(identity.email, "a@example.com");
        assert!(identity.key.starts_with("cline:") && !identity.key.contains("usr_1"));
    }

    #[test]
    fn kilo_wallets_skip_missing_personal_account_and_reject_negative() {
        let mut q = snapshot();
        let v = json!({"me":{"email":"B@Example.com"},"balance":null,"organizations":[{"id":"o","name":null,"balance":3}]});
        wallets(&mut q, "kilo", &v).unwrap();
        assert_eq!(q.meters.len(), 1);
        assert_eq!(q.meters[0].unit, "usd");
        assert_eq!(q.meters[0].label, "组织");
        assert_eq!(
            q.identity.unwrap().key,
            format!("kilo:{}", hash("b@example.com"))
        );
        let mut q = snapshot();
        assert!(wallets(&mut q, "kilo", &json!({"balance":-1})).is_err());
        let mut q = snapshot();
        wallets(&mut q, "kilo", &json!({"balance":null,"organizations":[]})).unwrap();
        assert_eq!(q.notice.as_deref(), Some("账户未返回余额"));
    }

    #[test]
    fn openrouter_key_limit_and_unlimited_key() {
        let mut q = snapshot();
        openrouter(
            &mut q,
            &json!({"data":{"label":"x","usage":2.5,"limit":10,"limit_remaining":7.5,"limit_reset":"monthly"}}),
        )
        .unwrap();
        let m = &q.meters[0];
        assert_eq!((m.used, m.total, m.remaining), (2.5, Some(10.), Some(7.5)));
        assert_eq!(m.label, "OpenRouter Key · 每月");
        let mut q = snapshot();
        openrouter(&mut q, &json!({"data":{"usage":1,"limit":null}})).unwrap();
        assert_eq!(q.meters[0].total, None);
        assert!(q.notice.is_some());
        assert!(openrouter(&mut snapshot(), &json!({"error":{}})).is_err());
    }

    #[test]
    fn openrouter_period_and_byok_usage_match_the_limit_scope() {
        for (reset, field, byok) in [
            ("daily", "usage_daily", "byok_usage_daily"),
            ("weekly", "usage_weekly", "byok_usage_weekly"),
            ("monthly", "usage_monthly", "byok_usage_monthly"),
        ] {
            let mut data = json!({"usage":900,"byok_usage":800,"limit":10,"limit_reset":reset,"limit_remaining":7.5});
            data[field] = json!(2.5);
            data[byok] = json!(1.);
            let mut quota = snapshot();
            openrouter(&mut quota, &json!({"data":data})).unwrap();
            assert_eq!(quota.meters[0].used, 2.5);
            data["include_byok_in_limit"] = json!(true);
            data["limit_remaining"] = Value::Null;
            let mut quota = snapshot();
            openrouter(&mut quota, &json!({"data":data})).unwrap();
            assert_eq!(
                (quota.meters[0].used, quota.meters[0].remaining),
                (3.5, Some(6.5))
            );
        }
        assert!(
            openrouter(
                &mut snapshot(),
                &json!({"data":{"usage":900,"limit":10,"limit_reset":"monthly"}})
            )
            .is_err()
        );
    }

    #[test]
    fn server_identifiers_cannot_change_the_route() {
        assert!(segment("org_01ABC-x").is_ok());
        for bad in ["", "../me", "a/b", "a?b", ".."] {
            assert!(segment(bad).is_err(), "{bad}");
        }
    }
}
