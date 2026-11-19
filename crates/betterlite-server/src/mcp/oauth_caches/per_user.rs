use async_trait::async_trait;

use crate::mcp::outbound_credentials::{OAuthToken, PER_USER_TOKEN_KEY_PREFIX};

use super::MCPPerUserTokenCache;

// The shared cache layer this store writes through; delete is the evict-and-broadcast path every peer worker observes.
#[async_trait]
pub trait PerUserTokenBackend: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn get(&self, key: &str) -> Result<Option<String>, Self::Error>;

    async fn set(&self, key: &str, value: &str, ttl: i64) -> Result<(), Self::Error>;

    async fn delete(&self, key: &str) -> Result<(), Self::Error>;
}

impl<B: PerUserTokenBackend> MCPPerUserTokenCache<B> {
    pub async fn get(&self, user_id: &str, server_id: &str) -> Option<String> {
        self.get_token(user_id, server_id)
            .await
            .map(|token| token.access_token)
    }

    // LiteLLM reads a cache or decrypt failure as a miss and debug-logs it, so an unavailable cache never fails a request.
    pub async fn get_token(&self, user_id: &str, server_id: &str) -> Option<OAuthToken> {
        let key = format!("{PER_USER_TOKEN_KEY_PREFIX}{user_id}:{server_id}");
        let encrypted = match self.backend.get(&key).await {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(
                    user_id,
                    server_id,
                    error = %error,
                    "MCPPerUserTokenCache.get failed"
                );
                return None;
            }
        };
        self.codec.decode(encrypted.as_deref()?)
    }

    pub async fn set(
        &self,
        user_id: &str,
        server_id: &str,
        access_token: &str,
        ttl: i64,
        identity_binding_proof: Option<&str>,
    ) {
        let key = format!("{PER_USER_TOKEN_KEY_PREFIX}{user_id}:{server_id}");
        let token = OAuthToken {
            access_token: access_token.to_owned(),
            expires_at: None,
            refresh_token: None,
            scopes: Vec::new(),
            identity_binding_proof: identity_binding_proof.map(str::to_owned),
        };
        let encrypted = match self.codec.encode(&token) {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(
                    user_id,
                    server_id,
                    error = %error,
                    "MCPPerUserTokenCache.set failed"
                );
                return;
            }
        };
        if let Err(error) = self.backend.set(&key, &encrypted, ttl).await {
            tracing::debug!(
                user_id,
                server_id,
                ttl,
                error = %error,
                "MCPPerUserTokenCache.set failed"
            );
        }
    }

    pub async fn delete(&self, user_id: &str, server_id: &str) {
        let key = format!("{PER_USER_TOKEN_KEY_PREFIX}{user_id}:{server_id}");
        if let Err(error) = self.backend.delete(&key).await {
            tracing::debug!(
                user_id,
                server_id,
                error = %error,
                "MCPPerUserTokenCache.delete failed"
            );
        }
    }
}
