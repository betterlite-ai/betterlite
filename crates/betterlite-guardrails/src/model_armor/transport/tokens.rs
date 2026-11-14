use async_trait::async_trait;
use secrecy::SecretString;

use super::error::ArmorTokenError;

// LiteLLM's access_token_provider seam; the Vertex credential lookup is wired outside betterlite-guardrails
#[async_trait]
pub trait ArmorTokens: Send + Sync {
    async fn access_token(&self) -> Result<(SecretString, Option<String>), ArmorTokenError>;
}
