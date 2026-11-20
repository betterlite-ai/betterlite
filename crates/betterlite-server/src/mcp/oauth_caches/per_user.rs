use std::time::Duration;

use crate::mcp::outbound_credentials::{OAuthToken, TokenCacheBackend, TokenKey};

use super::MCPPerUserTokenCache;

// One per-user token write: the bearer, its TTL and the identity binding proof.
#[derive(Clone, Copy)]
pub struct PerUserToken<'token> {
    pub access_token: &'token str,
    pub ttl_seconds: i64,
    pub identity_binding_proof: Option<&'token str>,
}

impl<B: TokenCacheBackend> MCPPerUserTokenCache<B> {
    pub async fn get(&self, user_id: &str, server_id: &str) -> Option<String> {
        self.get_token(user_id, server_id)
            .await
            .map(|token| token.access_token)
    }

    // The backend decodes the stored entry and reads a missing or undecryptable value as a miss.
    pub async fn get_token(&self, user_id: &str, server_id: &str) -> Option<OAuthToken> {
        self.backend
            .get(&per_user_token_key(user_id, server_id))
            .await
    }

    // The backend encrypts the token with its codec and reads every cache failure as a no-op.
    pub async fn set(&self, user_id: &str, server_id: &str, token: PerUserToken<'_>) {
        let stored = OAuthToken {
            access_token: token.access_token.to_owned(),
            expires_at: None,
            refresh_token: None,
            scopes: Vec::new(),
            identity_binding_proof: token.identity_binding_proof.map(str::to_owned),
        };
        let lifetime = Duration::from_secs(u64::try_from(token.ttl_seconds).unwrap_or_default());
        self.backend
            .set(&per_user_token_key(user_id, server_id), &stored, lifetime)
            .await;
    }

    pub async fn delete(&self, user_id: &str, server_id: &str) {
        self.backend
            .delete(&per_user_token_key(user_id, server_id))
            .await;
    }
}

fn per_user_token_key(user_id: &str, server_id: &str) -> TokenKey {
    TokenKey {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
    }
}
