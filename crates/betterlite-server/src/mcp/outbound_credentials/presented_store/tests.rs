#![cfg(test)]

use super::PresentedOAuthTokenStore;
use crate::mcp::outbound_credentials::{OAuthToken, OAuthTokenStore as _};

fn token() -> OAuthToken {
    OAuthToken {
        access_token: "preview-access".to_owned(),
        expires_at: None,
        refresh_token: None,
        scopes: vec!["read".to_owned()],
        identity_binding_proof: Some("proof".to_owned()),
    }
}

#[tokio::test]
async fn presented_store_serves_one_token_for_any_key() {
    let store = PresentedOAuthTokenStore { token: token() };
    let first = store.fetch("user-one", "server-one").await;
    let second = store.fetch("user-two", "server-two").await;
    assert_eq!(first, Ok(Some(token())));
    assert_eq!(second, Ok(Some(token())));
}
