mod redis_cache;
mod user_cache;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

use super::SsoReturnTargetError;
use crate::sso_pkce::VerifierEntropy;

pub const LOGIN_CODE_PREFIX: &str = "login_code:";
pub const LOGIN_CODE_TTL_SECONDS: f64 = 60.0;
const LOGIN_CODE_RANDOM_BYTES: usize = 32;

// serializes the plain token through this proxy, so the cache keeps its value while the secret's Debug stays redacted
#[derive(Serialize)]
struct ExposedToken(String);

impl From<&SecretString> for ExposedToken {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginCode<'code> {
    #[serde(serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedToken>>::serialize")]
    pub token: SecretString,
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
