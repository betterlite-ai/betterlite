use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

// LiteLLM's two validation TTLs: a known id is trusted for longer than an unknown one
pub const POSITIVE_TTL_SECONDS: f64 = 300.0;
pub const NEGATIVE_TTL_SECONDS: f64 = 60.0;

// LiteLLM's raw-cache key, which stores the verdict and never a row
pub const VALIDATION_KEY_PREFIX: &str = "end_user_validation:";

// the answer LiteLLM stores as the string "valid" or "invalid"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    Invalid,
}

impl Verdict {
    // only the two exact strings are hits, so any other cached value falls through to the database
    #[must_use]
    pub fn of_cached(cached: &Json) -> Option<Self> {
        match cached {
            Json::Text(text) if text == "valid" => Some(Self::Valid),
            Json::Text(text) if text == "invalid" => Some(Self::Invalid),
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => None,
        }
    }

    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }

    #[must_use]
    pub const fn ttl(self) -> WriteTtl {
        WriteTtl {
            seconds: match self {
                Self::Valid => POSITIVE_TTL_SECONDS,
                Self::Invalid => NEGATIVE_TTL_SECONDS,
            },
        }
    }
}

// the raw-cache read and write LiteLLM makes around its verdict; a failure escapes to the caller
#[async_trait]
pub trait ValidationCache: Send + Sync {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError>;
    async fn write(
        &self,
        cache_key: &str,
        value: &Json,
        ttl: WriteTtl,
    ) -> Result<(), AuthCacheError>;
}

#[async_trait]
impl<Tier: AuthCacheTier> ValidationCache for AuthCache<Tier> {
    async fn read(&self, cache_key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.async_get(cache_key, false).await)
    }

    async fn write(
        &self,
        cache_key: &str,
        value: &Json,
        ttl: WriteTtl,
    ) -> Result<(), AuthCacheError> {
        self.async_set(
            Some(cache_key),
            value,
            WriteOptions {
                ttl: Some(ttl),
                local_only: false,
                refresh_ttl: false,
            },
        )
        .await
    }
}
