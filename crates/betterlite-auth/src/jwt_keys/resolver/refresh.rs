use betterlite_models::decisions::Json;
use std::cmp::Ordering;
use std::ops::{Add as _, Sub as _};

use super::super::errors::KeyError;
use super::fetch::refresh_now;
use super::{
    JwksResolver, Refresh, STALE_CACHE_KEY_PREFIX, STALE_WRITTEN_AT_PREFIX,
    UNREACHABLE_BACKOFF_SECONDS, UNREACHABLE_CACHE_KEY_PREFIX,
};

pub(super) async fn refresh_or_serve_stale(
    resolver: &JwksResolver,
    cache_key: &str,
    ttl: f64,
    refresh: Refresh<'_>,
    log_context: &str,
) -> Result<Json, KeyError> {
    let stale_ttl = resolver.settings.public_key_stale_ttl;
    let unreachable =
        match refresh_or_record_outage(resolver, cache_key, ttl, stale_ttl, refresh).await {
            Ok(value) => return Ok(value),
            Err(error @ KeyError::Unreachable { .. }) => error,
            Err(other) => return Err(other),
        };
    if stale_ttl <= 0.0_f64 {
        return Err(unreachable);
    }
    let stale = resolver
        .cache
        .read_value(&format!("{STALE_CACHE_KEY_PREFIX}{cache_key}"))
        .await?
        .filter(|value| !matches!(value, Json::Null));
    let age = stale_copy_age(resolver, cache_key).await?;
    let Some(stale_value) = stale else {
        return Err(unreachable);
    };
    let Some(stale_age) = age else {
        return Err(unreachable);
    };
    let lifetime = ttl.add(stale_ttl);
    if stale_age
        .partial_cmp(&lifetime)
        .is_some_and(|ordering| ordering == Ordering::Greater)
    {
        return Err(unreachable);
    }
    let remaining = lifetime.sub(stale_age).max(0.0_f64);
    warn_stale_copy(log_context, cache_key, stale_age, remaining, &unreachable);
    Ok(stale_value)
}

// _refresh_or_record_outage: a failure is remembered for 30 s; a success writes fresh, stale, then time.
pub(super) async fn refresh_or_record_outage(
    resolver: &JwksResolver,
    cache_key: &str,
    ttl: f64,
    stale_ttl: f64,
    refresh: Refresh<'_>,
) -> Result<Json, KeyError> {
    let unreachable_key = format!("{UNREACHABLE_CACHE_KEY_PREFIX}{cache_key}");
    match resolver.cache.read_value(&unreachable_key).await? {
        None | Some(Json::Null) => {}
        Some(Json::Text(message)) => return Err(KeyError::Unreachable { message }),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(KeyError::Unreachable {
                message: "a previous JWKS fetch failed".to_owned(),
            });
        }
    }
    let refreshed = match refresh_now(resolver, refresh).await {
        Ok(value) => value,
        Err(KeyError::Unreachable { message }) => {
            resolver
                .cache
                .write_value(
                    &unreachable_key,
                    &Json::Text(message.clone()),
                    UNREACHABLE_BACKOFF_SECONDS,
                )
                .await?;
            return Err(KeyError::Unreachable { message });
        }
        Err(other) => return Err(other),
    };
    store_refreshed(resolver, cache_key, ttl, stale_ttl, refreshed).await
}

// a success writes fresh, stale, then time, so a failure leaves the entries written before it
pub(super) async fn store_refreshed(
    resolver: &JwksResolver,
    cache_key: &str,
    ttl: f64,
    stale_ttl: f64,
    refreshed: Json,
) -> Result<Json, KeyError> {
    resolver
        .cache
        .write_value(cache_key, &refreshed, ttl)
        .await?;
    if stale_ttl > 0.0_f64 {
        let lifetime = ttl.add(stale_ttl);
        resolver
            .cache
            .write_value(
                &format!("{STALE_CACHE_KEY_PREFIX}{cache_key}"),
                &refreshed,
                lifetime,
            )
            .await?;
        let written_at =
            serde_json::Number::from_f64(resolver.clock.now()).map_or(Json::Null, Json::Number);
        resolver
            .cache
            .write_value(
                &format!("{STALE_WRITTEN_AT_PREFIX}{cache_key}"),
                &written_at,
                lifetime,
            )
            .await?;
    }
    Ok(refreshed)
}

// _stale_copy_age: only a numeric epoch counts, and Python's bool is an int, so true is the epoch 1.0.
pub(super) async fn stale_copy_age(
    resolver: &JwksResolver,
    cache_key: &str,
) -> Result<Option<f64>, KeyError> {
    let written_at_key = format!("{STALE_WRITTEN_AT_PREFIX}{cache_key}");
    let written_at = resolver.cache.read_value(&written_at_key).await?;
    let epoch = match written_at {
        Some(Json::Number(number)) => number.as_f64(),
        Some(Json::Bool(flag)) => Some(if flag { 1.0_f64 } else { 0.0_f64 }),
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => None,
    };
    Ok(epoch.map(|written| resolver.clock.now().sub(written)))
}

fn warn_stale_copy(
    log_context: &str,
    cache_key: &str,
    age: f64,
    remaining: f64,
    unreachable: &KeyError,
) {
    tracing::warn!(
        target: "LiteLLM Proxy",
        "JWT Auth: identity provider unreachable, authenticating {log_context} against a stale JWKS copy \
         of {cache_key} (last refreshed {age:.0}s ago, stops being trusted in {remaining:.0}s). \
         Refresh failed: {unreachable}"
    );
}
