use betterlite_models::decisions::Json;

use super::super::errors::KeyError;
use super::{
    JwksResolver, Refresh, STALE_CACHE_KEY_PREFIX, STALE_WRITTEN_AT_PREFIX,
    UNREACHABLE_BACKOFF_SECONDS, UNREACHABLE_CACHE_KEY_PREFIX,
};

impl JwksResolver {
    // _refresh_or_serve_stale: only an unreachable IdP serves the last-known-good copy, inside ttl + stale_ttl.
    pub(super) async fn refresh_or_serve_stale(
        &self,
        cache_key: &str,
        ttl: f64,
        refresh: Refresh<'_>,
        log_context: &str,
    ) -> Result<Json, KeyError> {
        let stale_ttl = self.settings.public_key_stale_ttl;
        let unreachable = match self
            .refresh_or_record_outage(cache_key, ttl, stale_ttl, refresh)
            .await
        {
            Ok(value) => return Ok(value),
            Err(error @ KeyError::Unreachable { .. }) => error,
            Err(other) => return Err(other),
        };
        if stale_ttl <= 0.0_f64 {
            return Err(unreachable);
        }
        let stale = self
            .cache
            .get(&format!("{STALE_CACHE_KEY_PREFIX}{cache_key}"))
            .await?
            .filter(|value| !matches!(value, Json::Null));
        let age = self.stale_copy_age(cache_key).await?;
        let Some(stale) = stale else {
            return Err(unreachable);
        };
        let Some(age) = age else {
            return Err(unreachable);
        };
        let lifetime = ttl + stale_ttl;
        if age > lifetime {
            return Err(unreachable);
        }
        let remaining = (lifetime - age).max(0.0_f64);
        tracing::warn!(
            target: "LiteLLM Proxy",
            "JWT Auth: identity provider unreachable, authenticating {log_context} against a stale JWKS copy \
             of {cache_key} (last refreshed {age:.0}s ago, stops being trusted in {remaining:.0}s). \
             Refresh failed: {unreachable}"
        );
        Ok(stale)
    }

    // _refresh_or_record_outage: a failure is remembered for 30 s; a success writes fresh, stale, then time.
    async fn refresh_or_record_outage(
        &self,
        cache_key: &str,
        ttl: f64,
        stale_ttl: f64,
        refresh: Refresh<'_>,
    ) -> Result<Json, KeyError> {
        let unreachable_key = format!("{UNREACHABLE_CACHE_KEY_PREFIX}{cache_key}");
        if let Some(message) = self.stored_outage(&unreachable_key).await? {
            return Err(KeyError::Unreachable { message });
        }
        let refreshed = match self.refresh_now(refresh).await {
            Ok(value) => value,
            Err(KeyError::Unreachable { message }) => {
                self.cache
                    .set(
                        &unreachable_key,
                        &Json::Text(message.clone()),
                        UNREACHABLE_BACKOFF_SECONDS,
                    )
                    .await?;
                return Err(KeyError::Unreachable { message });
            }
            Err(other) => return Err(other),
        };
        self.store_refreshed(cache_key, ttl, stale_ttl, refreshed)
            .await
    }

    // a marker from an earlier outage answers for its 30 s; a stored non-text marker still counts as one
    async fn stored_outage(&self, key: &str) -> Result<Option<String>, KeyError> {
        let previous = self
            .cache
            .get(key)
            .await?
            .filter(|value| !matches!(value, Json::Null));
        Ok(previous.map(|value| match value {
            Json::Text(message) => message,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                "a previous JWKS fetch failed".to_owned()
            }
        }))
    }

    // a success writes fresh, stale, then time, so a failure leaves the entries written before it
    async fn store_refreshed(
        &self,
        cache_key: &str,
        ttl: f64,
        stale_ttl: f64,
        refreshed: Json,
    ) -> Result<Json, KeyError> {
        self.cache.set(cache_key, &refreshed, ttl).await?;
        if stale_ttl > 0.0_f64 {
            let lifetime = ttl + stale_ttl;
            self.cache
                .set(
                    &format!("{STALE_CACHE_KEY_PREFIX}{cache_key}"),
                    &refreshed,
                    lifetime,
                )
                .await?;
            let written_at =
                serde_json::Number::from_f64(self.clock.now()).map_or(Json::Null, Json::Number);
            self.cache
                .set(
                    &format!("{STALE_WRITTEN_AT_PREFIX}{cache_key}"),
                    &written_at,
                    lifetime,
                )
                .await?;
        }
        Ok(refreshed)
    }

    // _stale_copy_age: only a numeric epoch counts, and Python's bool is an int, so true is the epoch 1.0.
    async fn stale_copy_age(&self, cache_key: &str) -> Result<Option<f64>, KeyError> {
        let written_at_key = format!("{STALE_WRITTEN_AT_PREFIX}{cache_key}");
        let written_at = self.cache.get(&written_at_key).await?;
        let epoch = match written_at {
            Some(Json::Number(number)) => number.as_f64(),
            Some(Json::Bool(flag)) => Some(if flag { 1.0_f64 } else { 0.0_f64 }),
            Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => None,
        };
        Ok(epoch.map(|written| self.clock.now() - written))
    }
}
