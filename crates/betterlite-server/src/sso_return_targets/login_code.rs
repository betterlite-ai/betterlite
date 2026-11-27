use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_auth::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use serde::Serialize;
use thiserror::Error;

use super::SsoReturnTargetError;
use crate::sso_pkce::VerifierEntropy;

pub const LOGIN_CODE_PREFIX: &str = "login_code:";
pub const LOGIN_CODE_TTL_SECONDS: f64 = 60.0;
const LOGIN_CODE_RANDOM_BYTES: usize = 32;

#[derive(Debug, Clone, Serialize)]
pub struct LoginCode<'code> {
    pub token: &'code str,
    pub redirect_url: &'code str,
}

#[derive(Debug, Error)]
pub enum CacheWriteError {
    #[error("the user API key cache write failed")]
    UserCache(#[source] AuthCacheError),
    #[error("the Redis usage cache write failed")]
    RedisUsage(#[source] RedisCacheValuesError),
}

// secrets.token_urlsafe(32): 32 random bytes as unpadded base64url
pub fn generate_login_code<Entropy>(entropy: &Entropy) -> Result<String, SsoReturnTargetError>
where
    Entropy: VerifierEntropy,
{
    let mut bytes = [0_u8; LOGIN_CODE_RANDOM_BYTES];
    entropy
        .fill(&mut bytes)
        .map_err(SsoReturnTargetError::Entropy)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[async_trait]
pub trait LoginCodeCache: Send + Sync {
    async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError>;
}

#[async_trait]
impl<Tier: AuthCacheTier> LoginCodeCache for AuthCache<Tier> {
    async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError> {
        let options = WriteOptions {
            ttl: Some(WriteTtl {
                seconds: ttl_seconds,
            }),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(cache_key), code, options)
            .await
            .map_err(CacheWriteError::UserCache)
    }
}

#[async_trait]
impl LoginCodeCache for RedisCache {
    async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError> {
        self.values
            .async_set_cache(Some(cache_key), code, Some(ttl_seconds), false)
            .await
            .map_err(CacheWriteError::RedisUsage)?;
        Ok(())
    }
}

pub struct LoginCodeCaches<'caches, Redis, User> {
    pub redis: Option<&'caches Redis>,
    pub user: &'caches User,
}

impl<Redis, User> LoginCodeCaches<'_, Redis, User>
where
    Redis: LoginCodeCache,
    User: LoginCodeCache,
{
    // LiteLLM writes the login code to the Redis usage cache when one is configured and the user cache otherwise
    pub(crate) async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError> {
        match self.redis {
            Some(redis) => redis.store_login_code(cache_key, code, ttl_seconds).await,
            None => {
                self.user
                    .store_login_code(cache_key, code, ttl_seconds)
                    .await
            }
        }
    }
}
