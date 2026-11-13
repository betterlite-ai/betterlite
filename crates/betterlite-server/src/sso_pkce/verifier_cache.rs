use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_models::decisions::Json;
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

// A cache read failure is a miss and a delete failure is a warning, as both LiteLLM caches do
#[async_trait]
pub trait VerifierCache: Send + Sync {
    fn kind(&self) -> PkceCacheKind;
    async fn read(&self, cache_key: &str) -> Option<PkceCacheEntry>;
    async fn delete(&self, cache_key: &str);
}

#[async_trait]
impl<T: AuthCacheTier> VerifierCache for AuthCache<T> {
    pub(crate) fn kind(&self) -> PkceCacheKind {
        PkceCacheKind::UserCache
    }

    async fn read(&self, cache_key: &str) -> Option<PkceCacheEntry> {
        self.async_get(cache_key, false)
            .await
            .and_then(|value| stored_entry(&value))
    }

    async fn delete(&self, cache_key: &str) {
        if let Err(error) = self.tier(cache_key).async_delete(cache_key).await {
            tracing::warn!(
                %error,
                "PKCE: failed to delete the verifier cache key (best-effort cleanup)"
            );
        }
    }
}

// Both cache tiers read a stored null as a miss, so only a present non-null value reaches this match
fn stored_entry(value: &Json) -> Option<PkceCacheEntry> {
    match value {
        Json::Null => None,
        Json::Object(fields) => Some(object_entry(fields.get("code_verifier"))),
        Json::Text(text) => Some(legacy_entry(text)),
        Json::Bool(_) => Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::Boolean)),
        Json::Number(_) => Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::Number)),
        Json::List(_) => Some(PkceCacheEntry::UnexpectedShape(PkceCacheShape::List)),
    }
}

fn object_entry(field: Option<&Json>) -> PkceCacheEntry {
    match field {
        Some(Json::Text(text)) if !text.is_empty() => {
            PkceCacheEntry::Verifier(SecretString::from(text.clone()))
        }
        Some(Json::Text(_)) | Some(Json::Null) => PkceCacheEntry::EmptyVerifier,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) | None => {
            PkceCacheEntry::UnexpectedShape(PkceCacheShape::Object)
        }
    }
}

fn legacy_entry(text: &str) -> PkceCacheEntry {
    if text.is_empty() {
        PkceCacheEntry::EmptyVerifier
    } else {
        PkceCacheEntry::LegacyVerifier(SecretString::from(text.to_owned()))
    }
}

pub struct PkceCaches<'a, Redis, User> {
    pub redis: Option<&'a Redis>,
    pub user: &'a User,
}

impl<Redis, User> PkceCaches<'_, Redis, User>
where
    Redis: VerifierCache,
    User: VerifierCache,
{
    // LiteLLM reads the Redis usage cache when one is configured and the user cache otherwise
    pub(crate) async fn read_verifier(&self, cache_key: &str) -> Option<PkceCacheEntry> {
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

    pub(crate) fn kind(&self) -> PkceCacheKind {
        match self.redis {
            Some(redis) => redis.kind(),
            None => self.user.kind(),
        }
    }
}
