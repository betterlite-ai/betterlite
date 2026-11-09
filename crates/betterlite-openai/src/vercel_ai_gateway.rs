pub mod chat;
pub mod embedding;
pub mod error;
pub mod models;
pub mod models_error;
pub mod provider_info;

mod tests;

use secrecy::SecretString;

pub const DEFAULT_API_BASE: &str = "https://ai-gateway.vercel.sh/v1";

#[derive(Debug, Default)]
pub struct VercelAiGatewayEnvironment {
    pub vercel_ai_gateway_api_base: Option<String>,
    pub vercel_ai_gateway_api_key: Option<SecretString>,
    pub vercel_oidc_token: Option<SecretString>,
}
