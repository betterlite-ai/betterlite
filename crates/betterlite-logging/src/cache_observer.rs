use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use futures_util::future::join_all;
use http::HeaderMap;
use serde::Deserialize;
use serde::Serialize;
use url::Url;

pub mod observer;
mod tests;

pub const RETENTION_SECONDS: f64 = 86_400.0;
pub const LOCAL_COPY_SECONDS: f64 = 1.0;

const CACHE_KEY_PREFIX: &str = "prompt-cache-observation";
const FINGERPRINT_LENGTH: usize = 64;
const ANTHROPIC_PROVIDER: &str = "anthropic";
const ANTHROPIC_MESSAGES_CALL: &str = "anthropic_messages";
const SUCCESS_STATUS_CODE: u16 = 200;

// the wire request is what LiteLLM hands parse_observed_cache: the URL, the headers and the raw body
#[derive(Clone, Copy, Debug)]
pub struct WireRequest<'wire> {
    pub url: &'wire Url,
    pub headers: &'wire HeaderMap,
    pub body: &'wire [u8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedPrefix {
    pub fingerprint: String,
    pub fingerprints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedCache {
    pub prefix: ObservedPrefix,
    pub ttl_seconds: u32,
    pub scope: String,
    pub cached_tokens: i64,
    pub cache_creation_tokens: i64,
}

// the decoder lives in betterlite-anthropic, which depends on this crate, so it arrives through a port
pub trait ObservationParser: Send + Sync {
    fn parse(
        &self,
        wire: &WireRequest<'_>,
        response: &Json,
        caller_key_hash: &str,
        deployment_id: &str,
    ) -> Option<ObservedCache>;
}

// DualCache lives in betterlite-cache, which depends on this crate, so the observer takes a port
#[async_trait]
pub trait ObservationCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<Json>;
    async fn set(&self, key: &str, value: &Json, ttl_seconds: f64, local_only: bool);
    fn has_redis(&self) -> bool;
}

// LiteLLM's frozen strict model; the cache holds it as this JSON object or as its serialized string
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheObservation {
    pub fingerprint: String,
    pub cached_tokens: i64,
    pub observed_at: f64,
    pub expires_at: f64,
}

impl CacheObservation {
    #[must_use]
    pub fn validated(self) -> Option<Self> {
        let fingerprint_ok = self.fingerprint.len() == FINGERPRINT_LENGTH
            && self
                .fingerprint
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        let counts_ok = self.cached_tokens > 0;
        let times_ok = self.observed_at.is_finite()
            && self.observed_at >= 0.0
            && self.expires_at.is_finite()
            && self.expires_at >= 0.0;
        (fingerprint_ok && counts_ok && times_ok).then_some(self)
    }
}

// LiteLLM defaults this callable to time.time and every test injects its own
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

async fn read_exact(
    cache: &dyn ObservationCache,
    scope: &str,
    fingerprint: &str,
) -> Option<CacheObservation> {
    let key = format!("{CACHE_KEY_PREFIX}:{scope}:{fingerprint}");
    let value = cache.get(&key).await?;
    let observation = match &value {
        Json::Text(text) => serde_json::from_str::<CacheObservation>(text).ok()?,
        Json::Object(_) => serde_json::from_value::<CacheObservation>(value.clone()).ok()?,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => return None,
    };
    observation
        .validated()
        .filter(|found| found.fingerprint == fingerprint)
}

// the exact fingerprint wins, the lookback ones cover a changed tail, and a stale entry beats none
#[must_use]
pub async fn lookup(
    cache: &dyn ObservationCache,
    scope: &str,
    prefix: &ObservedPrefix,
    now: Option<f64>,
) -> Option<CacheObservation> {
    let checked_at = now.unwrap_or_else(unix_now);
    let exact = read_exact(cache, scope, &prefix.fingerprint).await;
    if let Some(found) = &exact
        && found.expires_at > checked_at
    {
        return Some(found.clone());
    }
    let older = join_all(
        prefix
            .fingerprints
            .iter()
            .skip(1)
            .map(|fingerprint| read_exact(cache, scope, fingerprint)),
    )
    .await;
    let observations = exact
        .into_iter()
        .chain(older.into_iter().flatten())
        .collect::<Vec<CacheObservation>>();
    observations
        .iter()
        .find(|found| found.expires_at > checked_at)
        .or(observations.first())
        .cloned()
}
