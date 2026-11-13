mod tests;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::TimeDelta;

use super::{
    InvalidatableOAuthTokenStore, OAuthToken, OAuthTokenStore, TokenCacheBackend, TokenKey,
    TokenStoreUnavailable, WallClock,
};

pub struct CachedOAuthTokenStore<Inner, Backend> {
    pub inner: Inner,
    pub default_ttl: Duration,
    pub expiry_skew: Duration,
    pub backend: Backend,
    pub clock: Arc<dyn WallClock>,
}

impl<Inner, Backend> CachedOAuthTokenStore<Inner, Backend> {
    fn ttl(&self, token: &OAuthToken) -> Duration {
        let Some(expires_at) = token.expires_at else {
            return self.default_ttl;
        };
        let Ok(skew) = TimeDelta::from_std(self.expiry_skew) else {
            return Duration::ZERO;
        };
        expires_at
            .checked_sub_signed(skew)
            .map_or(Duration::ZERO, |deadline| {
                deadline
                    .signed_duration_since(self.clock.now())
                    .to_std()
                    .unwrap_or_default()
            })
    }
}

#[async_trait]
impl<Inner, Backend> OAuthTokenStore for CachedOAuthTokenStore<Inner, Backend>
where
    Inner: OAuthTokenStore,
    Backend: TokenCacheBackend,
{
    async fn fetch(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        let key = TokenKey {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        };
        if let Some(hit) = self.backend.get(&key).await {
            return Ok(Some(hit));
        }
        let Some(token) = self.inner.fetch(user_id, server_id).await? else {
            self.backend.delete(&key).await;
            return Ok(None);
        };
        self.backend.set(&key, &token, self.ttl(&token)).await;
        Ok(Some(token))
    }
}

#[async_trait]
impl<Inner, Backend> InvalidatableOAuthTokenStore for CachedOAuthTokenStore<Inner, Backend>
where
    Inner: OAuthTokenStore,
    Backend: TokenCacheBackend,
{
    async fn invalidate(&self, user_id: &str, server_id: &str) {
        let key = TokenKey {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
        };
        self.backend.delete(&key).await;
    }
}
