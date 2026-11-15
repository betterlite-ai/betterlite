use std::error::Error;

use async_trait::async_trait;
use secrecy::SecretString;

// LiteLLM's access_token_provider seam; the Vertex credential lookup is wired outside betterlite-guardrails
#[async_trait]
pub trait ArmorTokens: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn access_token(&self) -> Result<(SecretString, Option<String>), Self::Error>;
}
