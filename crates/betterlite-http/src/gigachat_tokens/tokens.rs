use async_trait::async_trait;
use secrecy::SecretString;

use crate::gigachat_tokens::error::GigachatAuthError;
use crate::gigachat_tokens::request::GigachatTokenRequest;

#[async_trait]
pub trait GigachatTokens: Send + Sync {
    async fn access_token(
        &self,
        request: &GigachatTokenRequest,
    ) -> Result<SecretString, GigachatAuthError>;
}
