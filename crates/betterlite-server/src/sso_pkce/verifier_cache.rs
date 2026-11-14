use std::fmt::Display;

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use secrecy::SecretString;

use super::PkceCacheShape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkceCacheKind {
    RedisUsage,
    UserCache,
}

#[derive(Debug, Clone)]
pub enum PkceCacheEntry {
    Verifier(SecretString),
    LegacyVerifier(SecretString),
    EmptyVerifier,
    UnexpectedShape(PkceCacheShape),
}

#[derive(Debug, Clone)]
pub enum CacheRead {
    Found(PkceCacheEntry),
    Miss,
}

// A cache read failure is a miss and a delete failure is a warning, as both LiteLLM caches do
#[async_trait]
pub trait VerifierCache: Send + Sync {
    fn kind(&self) -> PkceCacheKind;
    async fn read(&self, cache_key: &str) -> CacheRead;
    async fn delete(&self, cache_key: &str);
}

#[async_trait]
impl<T: AuthCacheTier> VerifierCache for AuthCache<T> {
    fn kind(&self) -> PkceCacheKind {
        PkceCacheKind::UserCache
    }

    async fn read(&self, cache_key: &str) -> CacheRead {
        let stored = self.async_get(cache_key, false).await;
        stored
            .and_then(|value| stored_entry(&value))
            .map_or(CacheRead::Miss, CacheRead::Found)
    }

    async fn delete(&self, cache_key: &str) {
        warn_when_delete_fails(self.tier(cache_key).async_delete(cache_key).await);
    }
}

#[async_trait]
impl VerifierCache for RedisCache {
    fn kind(&self) -> PkceCacheKind {
        PkceCacheKind::RedisUsage
    }

    async fn read(&self, cache_key: &str) -> CacheRead {
        match self.async_get_cache(cache_key).await {
            Ok(stored) => stored
                .and_then(|value| stored_entry(&json_from_cached(value)))
                .map_or(CacheRead::Miss, CacheRead::Found),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "PKCE: the Redis usage cache read failed; treating it as a miss"
                );
                CacheRead::Miss
            }
        }
    }

    async fn delete(&self, cache_key: &str) {
        warn_when_delete_fails(self.admin.async_delete_cache(cache_key).await);
    }
}

fn warn_when_delete_fails<T, E>(outcome: Result<T, E>)
where
    E: Display,
{
    if let Err(error) = outcome {
        tracing::warn!(
            %error,
            "PKCE: failed to delete the verifier cache key (best-effort cleanup)"
        );
    }
}

// betterlite-cache's own CachedValue conversion is crate-private, so this reader maps the typed value itself
pub(super) fn json_from_cached(value: CachedValue) -> Json {
    match value {
        CachedValue::Null => Json::Null,
        CachedValue::Boolean(flag) => Json::Bool(flag),
        CachedValue::Number(number) => Json::Number(number),
        CachedValue::Text(text) => Json::Text(text),
        CachedValue::Array(items) => Json::List(items.into_iter().map(json_from_cached).collect()),
        CachedValue::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, entry)| (key, json_from_cached(entry)))
                .collect(),
        ),
    }
}

// Both cache tiers read a stored null as a miss, so only a present non-null value reaches this match
pub(super) fn stored_entry(value: &Json) -> Option<PkceCacheEntry> {
    let shape = match value {
        Json::Null => return None,
        Json::Object(fields) => return Some(object_entry(fields.get("code_verifier"))),
        Json::Text(text) => return Some(text_entry(text)),
        Json::Bool(_) => PkceCacheShape::Boolean,
        Json::Number(_) => PkceCacheShape::Number,
        Json::List(_) => PkceCacheShape::List,
    };
    Some(PkceCacheEntry::UnexpectedShape(shape))
}

fn object_entry(verifier: Option<&Json>) -> PkceCacheEntry {
    match verifier {
        Some(Json::Text(text)) if !text.is_empty() => {
            PkceCacheEntry::Verifier(SecretString::from(text.clone()))
        }
        Some(Json::Text(_) | Json::Null) => PkceCacheEntry::EmptyVerifier,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) | None => {
            PkceCacheEntry::UnexpectedShape(PkceCacheShape::Object)
        }
    }
}

fn text_entry(text: &str) -> PkceCacheEntry {
    if text.is_empty() {
        PkceCacheEntry::EmptyVerifier
    } else {
        PkceCacheEntry::LegacyVerifier(SecretString::from(text.to_owned()))
    }
}

pub struct PkceCaches<'caches, Redis, User> {
    pub redis: Option<&'caches Redis>,
    pub user: &'caches User,
}

impl<Redis, User> PkceCaches<'_, Redis, User>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    // LiteLLM reads the Redis usage cache when one is configured and the user cache otherwise
    pub(crate) async fn read_verifier(&self, cache_key: &str) -> CacheRead {
        match self.redis {
            Some(redis) => redis.read(cache_key).await,
            None => self.user.read(cache_key).await,
        }
    }

    pub async fn delete_verifier(&self, cache_key: &str) {
        match self.redis {
            Some(redis) => redis.delete(cache_key).await,
            None => self.user.delete(cache_key).await,
        }
    }
}
