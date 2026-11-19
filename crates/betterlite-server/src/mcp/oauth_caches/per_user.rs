use std::time::Duration;

use crate::mcp::outbound_credentials::{OAuthToken, TokenCacheBackend, TokenKey};

use super::MCPPerUserTokenCache;

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
    pub async fn set(
        &self,
        user_id: &str,
        server_id: &str,
        access_token: &str,
        ttl: i64,
        identity_binding_proof: Option<&str>,
    ) {
        let token = OAuthToken {
            access_token: access_token.to_owned(),
            expires_at: None,
            refresh_token: None,
            scopes: Vec::new(),
            identity_binding_proof: identity_binding_proof.map(str::to_owned),
        };
        let lifetime = Duration::from_secs(u64::try_from(ttl).unwrap_or_default());
        self.backend
            .set(&per_user_token_key(user_id, server_id), &token, lifetime)
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
