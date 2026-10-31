//! Native SSH quotas use credentials on the owning host, while this module
//! reuses the local scanner's identity extraction and provider quota parsers.
use super::*;
use crate::native_quota_transport::{Projection, Remote};

fn candidate_from_projection(row: &NativeConnection, projection: &Projection) -> Result<Candidate> {
    let doc = Document {
        root: row.root.clone(),
        path: row.config_path.clone(),
        bytes: serde_json::to_vec(&projection.value)?,
    };
    let mut result = if row.kind == "api" {
        // API identities do not come from the token. The exact fingerprint and
        // freshly read native route guard the reference; prepaid logins below
        // ask their profile endpoint for the server user.
        candidate(
            &row.provider,
            &row.machine_id,
            &doc,
            &row.selector,
            "api",
            &projection.token_stub,
        )
    } else {
        let mut candidates = extract(&row.provider, &row.machine_id, &doc)?;
        let mut documents = vec![doc];
        for companion in projection.companions.iter().take(2) {
            documents.push(Document {
                root: row.root.clone(),
                path: companion.path.clone(),
                bytes: serde_json::to_vec(&companion.value)?,
            });
        }
        enrich_identities(&row.provider, &documents, &mut candidates);
        candidates
            .into_iter()
            .find(|c| c.connection.id == row.id)
            .context("原生凭据已移除")?
    };
    result.fingerprint = projection.fingerprint.clone();
    if !projection.base_url.is_empty() {
        result.connection.base_url = url(&projection.base_url);
    }
    Ok(result)
}

fn projected_identity(candidate: &mut Candidate, remote: &Remote) -> Result<()> {
    let row = &mut candidate.connection;
    if prepaid_account(row) {
        let identity = crate::subscriptions::prepaid_identity_using(
            &row.provider,
            &candidate.secret,
            |request| remote.request(request),
        )?;
        row.identity_key = Some(identity.key);
        row.email = public_text(&identity.email);
        return Ok(());
    }
    if row.provider == "copilot" {
        let identity = crate::upstream::probe_using("copilot", &candidate.secret, "", |request| {
            remote.request(request)
        })?;
        bind_github_identity(row, &identity);
        return Ok(());
    }
    if let Some(kind) = probe_kind(row) {
        let identity = if kind == "gemini" {
            crate::upstream::gemini_identity_using(
                candidate
                    .upstream_auth
                    .as_ref()
                    .context("上游登录凭据不可用")?,
                |request| remote.request(request),
            )?
        } else {
            crate::upstream::probe_using(kind, &candidate.secret, &row.base_url, |request| {
                remote.request(request)
            })?
        };
        row.identity_key = Some(if kind == "kimi" {
            identity.upstream.clone()
        } else {
            crate::upstream::client_identity(&row.provider, &row.selector, kind, &identity.subject)
        });
        row.upstream_identity = Some(identity.upstream);
        if !identity.email.is_empty() {
            row.email = public_text(&identity.email);
        }
    }
    Ok(())
}

fn verify(row: &NativeConnection, candidate: &Candidate, fingerprint: &str) -> Result<()> {
    ensure!(
        candidate.connection.identity_key == row.identity_key,
        "原生登录账户已变化，正在重新识别"
    );
    ensure!(
        row.identity_key.is_some() || candidate.fingerprint == fingerprint,
        "凭据已变化，正在重新识别"
    );
    ensure!(
        candidate.connection.status != "stale",
        "{}",
        stale_note(&row.provider)
    );
    // An API route change is a new native connection choice even if the key
    // bytes did not change. Never automatically retry it in another region.
    if row.kind == "api" {
        ensure!(
            native_subscription(row) == native_subscription(&candidate.connection),
            "原生 API 配置已变化，正在重新识别"
        );
    }
    Ok(())
}

pub(super) fn read(
    store: &crate::store::Store,
    settings: &Settings,
    account: &Account,
    source: &Source,
    row: &NativeConnection,
) -> Result<QuotaSnapshot> {
    let result = read_inner_using(store, settings, account, source, row, |source| {
        crate::quota::read(source, settings)
    });
    if upstream_kind(row).is_some() {
        result.map_err(|error| crate::upstream::expired(error, &row.provider))
    } else {
        result
    }
}

fn read_inner_using(
    store: &crate::store::Store,
    settings: &Settings,
    account: &Account,
    source: &Source,
    row: &NativeConnection,
    native_codex: impl FnOnce(&Source) -> Result<QuotaSnapshot>,
) -> Result<QuotaSnapshot> {
    let remote = Remote::inspect(settings, source, row)?;
    let mut current = candidate_from_projection(row, &remote.projection)?;
    let fingerprint = stored_fingerprint(store, &row.id).context("原生凭据尚未识别")?;
    ensure!(
        current.connection.status != "stale",
        "{}",
        stale_note(&row.provider)
    );
    // SSH never starts Kiro to refresh or borrows another native directory.
    if row.provider == "kiro" {
        ensure!(
            current.connection.token_expired_at.is_none(),
            "{}",
            crate::subscriptions::kiro::RELOGIN
        );
    }
    projected_identity(&mut current, &remote)?;
    verify(row, &current, &fingerprint)?;
    if crate::subscriptions::trae::family(&row.provider)
        && let Some(recent) = trae_recent(
            store,
            account,
            row,
            current.connection.token_expired_at.is_some(),
        )?
    {
        return Ok(recent);
    }
    let mut query = source.clone();
    query.subscription = native_subscription(&current.connection);
    let kind = upstream_kind(row).unwrap_or(&row.provider);
    let mut snapshot = match kind {
        "gemini" => crate::subscriptions::gemini::read_using(
            &query,
            current
                .upstream_auth
                .as_ref()
                .context("上游登录凭据不可用")?,
            row.upstream_identity.as_deref(),
            |request| remote.request(request),
        )?,
        "claude" => {
            let value =
                crate::upstream::usage_using(&current.secret, |request| remote.request(request))?;
            let mut upstream = query.clone();
            upstream.provider = "claude".into();
            crate::quota::normalize(&upstream, &value, "live")?
        }
        "codex" if row.provider == "codex" => {
            // Preserve Codex's established remote HOME/app-server path and its
            // own refresh behavior. Only upstream copies use ephemeral auth.
            native_codex(&query)?
        }
        "codex" => {
            let value = remote.codex(&source.codex_binary)?;
            let mut upstream = query.clone();
            upstream.provider = "codex".into();
            crate::quota::normalize(&upstream, &value, "live")?
        }
        "copilot" if upstream_kind(row).is_some() => {
            let mut upstream = query.clone();
            upstream.provider = "copilot".into();
            upstream.subscription = Some(crate::subscriptions::SubscriptionConfig {
                mode: "github".into(),
                ..Default::default()
            });
            crate::subscriptions::read_native_using(&upstream, &current.secret, |request| {
                remote.request(request)
            })?
        }
        _ => {
            let first =
                crate::subscriptions::read_native_using(&query, &current.secret, |request| {
                    remote.request(request)
                });
            if first.is_err() && row.provider == "zcode" && row.kind == "login" {
                zcode_fallback(settings, &query, row, first)?
            } else {
                first?
            }
        }
    };
    // Wallet and Copilot endpoints name the server user too. A provider-side
    // account mismatch cannot be hidden by unchanged native file metadata.
    if prepaid_account(row) || row.provider == "copilot" {
        ensure!(
            snapshot.identity.as_ref().map(|i| &i.key) == row.identity_key.as_ref(),
            "查询期间原生账户身份已变化"
        );
    } else if kind == "copilot" {
        ensure!(
            row.upstream_identity.as_ref().is_some_and(|expected| {
                snapshot
                    .identity
                    .as_ref()
                    .is_some_and(|identity| crate::upstream::copilot_matches(identity, expected))
            }),
            "查询期间原生账户身份已变化"
        );
    }
    if row.provider == "codex" {
        let after = Remote::inspect(settings, source, row)?;
        let current = candidate_from_projection(row, &after.projection)?;
        ensure!(
            current.connection.identity_key == row.identity_key,
            "查询期间原生账户身份已变化"
        );
    } else {
        remote.verify()?;
    }
    if snapshot.plan.is_none() && !current.plan.is_empty() {
        snapshot.plan = Some(current.plan);
    }
    snapshot.source_id = source.id.clone();
    snapshot.account_id = source.account_id.clone();
    snapshot.provider = source.provider.clone();
    snapshot.name = source.name.clone();
    Ok(snapshot)
}

fn zcode_fallback(
    settings: &Settings,
    source: &Source,
    login: &NativeConnection,
    first: Result<QuotaSnapshot>,
) -> Result<QuotaSnapshot> {
    let region = source.subscription.as_ref().map(|c| c.mode.as_str());
    let Some(key) = settings.native_connections.iter().find(|key| {
        key.provider == "zcode"
            && key.machine_id == login.machine_id
            && key.root == login.root
            && key.kind == "api"
            && key.has_credential
            && login.identity_key.is_some()
            && key.identity_key == login.identity_key
            && zcode_region(&key.base_url) == region
    }) else {
        return first;
    };
    let remote = Remote::inspect(settings, source, key)?;
    let candidate = candidate_from_projection(key, &remote.projection)?;
    // A remembered association alone is not identity proof. The freshly read
    // key must still identify this exact login, as in the local fallback.
    if candidate.connection.identity_key != login.identity_key {
        return first;
    }
    let result = crate::subscriptions::read_native_using(source, &candidate.secret, |request| {
        remote.request(request)
    })?;
    remote.verify()?;
    Ok(result)
}

/// Opaque logins need a profile identity to become a quota account on first
/// discovery. The remote profile is read with the same transport and parser;
/// discovery's existing document transport is otherwise unchanged.
pub(super) fn probe_scan_identities(
    store: &crate::store::Store,
    settings: &Settings,
    candidates: &mut [Candidate],
) {
    for c in candidates.iter_mut().filter(|c| {
        c.connection.machine_id != "local"
            && !c.secret.is_empty()
            && (c.connection.status != "stale" || upstream_kind(&c.connection) == Some("gemini"))
            && (prepaid_account(&c.connection)
                || probe_kind(&c.connection).is_some()
                || needs_github_identity(c))
    }) {
        if reuse_verified_identity(store, settings, c) {
            continue;
        }
        let gemini = upstream_kind(&c.connection) == Some("gemini");
        if gemini && c.connection.token_expired_at.is_some() {
            retain_gemini_identity(store, settings, c);
            continue;
        }
        let source = Source {
            provider: c.connection.provider.clone(),
            path: c.connection.root.clone(),
            host_id: Some(c.connection.machine_id.clone()),
            ..Default::default()
        };
        let result = (|| -> Result<Candidate> {
            let remote = Remote::inspect(settings, &source, &c.connection)?;
            ensure!(
                remote.projection.fingerprint == c.fingerprint,
                "凭据已变化，正在重新识别"
            );
            let mut projected = candidate_from_projection(&c.connection, &remote.projection)?;
            projected_identity(&mut projected, &remote)?;
            remote.verify()?;
            Ok(projected)
        })();
        match result {
            Ok(projected) => {
                c.connection.identity_key = projected.connection.identity_key;
                c.connection.upstream_identity = projected.connection.upstream_identity;
                c.connection.email = projected.connection.email;
                c.connection.base_url = projected.connection.base_url;
            }
            Err(error) => {
                if c.connection.provider == "copilot" {
                    retain_github_identity(store, settings, c);
                } else if gemini {
                    retain_gemini_identity(store, settings, c);
                }
                c.connection.note = error.to_string();
            }
        }
    }
}

/// Keep the last verified pool and snapshot visible during native token
/// expiry/failure. The old fingerprint remains the proof: a changed token is
/// still unverified and must pass fresh profile identity plus every query pin.
fn retain_gemini_identity(store: &crate::store::Store, settings: &Settings, c: &mut Candidate) {
    let old = settings.native_connections.iter().find(|old| {
        old.id == c.connection.id
            && old.identity_key.is_some()
            && old
                .upstream_identity
                .as_deref()
                .is_some_and(|id| id.starts_with("google-gemini-cli:"))
    });
    if let Some(old) = old
        && let Some(fingerprint) = stored_fingerprint(store, &old.id)
    {
        copy_identity(c, old);
        c.fingerprint = fingerprint;
    }
}

#[cfg(test)]
mod tests;
