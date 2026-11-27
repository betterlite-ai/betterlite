use async_trait::async_trait;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_auth::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

pub const LOGIN_CODE_PREFIX: &str = "login_code:";
pub const LOGIN_CODE_TTL_SECONDS: f64 = 60.0;

pub struct LoginCode {
    pub token: SecretString,
    pub redirect_url: String,
}

// the cache document LiteLLM writes: {"token": ..., "redirect_url": ...}, with the token read only here
#[derive(Serialize)]
struct LoginCodePayload<'code> {
    token: &'code str,
    redirect_url: &'code str,
}

impl LoginCode {
    fn payload(&self) -> LoginCodePayload<'_> {
        LoginCodePayload {
            token: self.token.expose_secret(),
            redirect_url: &self.redirect_url,
        }
    }
}

#[derive(Debug, Error)]
pub enum LoginCodeCacheError {
    #[error("the Redis login code cache write failed")]
    Redis(#[source] RedisCacheValuesError),
    #[error("the local login code cache write failed")]
    Local(#[source] AuthCacheError),
}

#[async_trait]
pub trait LoginCodeCache: Send + Sync {
    async fn store(
        &self,
        key: &str,
        value: &LoginCode,
        ttl_seconds: f64,
    ) -> Result<(), LoginCodeCacheError>;
}

#[async_trait]
impl LoginCodeCache for RedisCache {
    async fn store(
        &self,
        key: &str,
        value: &LoginCode,
        ttl_seconds: f64,
    ) -> Result<(), LoginCodeCacheError> {
        self.values
            .async_set_cache(Some(key), &value.payload(), Some(ttl_seconds), false)
            .await
            .map(|_stored| ())
            .map_err(LoginCodeCacheError::Redis)
    }
}

#[async_trait]
impl<T: AuthCacheTier> LoginCodeCache for AuthCache<T> {
    async fn store(
        &self,
        key: &str,
        value: &LoginCode,
        ttl_seconds: f64,
    ) -> Result<(), LoginCodeCacheError> {
        self.async_set(
            Some(key),
            &value.payload(),
            WriteOptions {
                ttl: Some(WriteTtl {
                    seconds: ttl_seconds,
                }),
                local_only: false,
                refresh_ttl: false,
            },
        )
        .await
        .map_err(LoginCodeCacheError::Local)
    }
}

pub struct LoginCaches<'caches, Redis, User> {
    pub redis: Option<&'caches Redis>,
    pub user: &'caches User,
}

impl<Redis, User> LoginCaches<'_, Redis, User>
where
    Redis: LoginCodeCache,
    User: LoginCodeCache,
{
    // _sso_return_to_redirect writes the configured Redis usage cache and the user API key cache otherwise
    pub(super) async fn store(
        &self,
        key: &str,
        value: &LoginCode,
        ttl_seconds: f64,
    ) -> Result<(), LoginCodeCacheError> {
        match self.redis {
            Some(redis) => redis.store(key, value, ttl_seconds).await,
            None => self.user.store(key, value, ttl_seconds).await,
        }
    }
}
