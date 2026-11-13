mod tests;

use async_trait::async_trait;

use super::{OAuthToken, OAuthTokenStore, TokenStoreUnavailable};

pub struct PresentedOAuthTokenStore {
    pub token: OAuthToken,
}

#[async_trait]
impl OAuthTokenStore for PresentedOAuthTokenStore {
    async fn fetch(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        Ok(Some(self.token.clone()))
    }
}
