use secrecy::{ExposeSecret as _, SecretString};

use crate::vercel_ai_gateway::{DEFAULT_API_BASE, VercelAiGatewayEnvironment};

#[derive(Debug)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

// an empty string falls through every "a or b or c" chain, as the compatibility policy says
#[must_use]
pub fn resolve_api_base(
    api_base: Option<&str>,
    environment: &VercelAiGatewayEnvironment,
) -> String {
    api_base
        .filter(|base| !base.is_empty())
        .or_else(|| {
            environment
                .vercel_ai_gateway_api_base
                .as_deref()
                .filter(|base| !base.is_empty())
        })
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned()
}

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &VercelAiGatewayEnvironment,
) -> ProviderInfo {
    let resolved_base = resolve_api_base(api_base, environment);
    let resolved_key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or_else(|| {
            environment
                .vercel_ai_gateway_api_key
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .or_else(|| {
            environment
                .vercel_oidc_token
                .as_ref()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .cloned();
    ProviderInfo {
        api_base: resolved_base,
        api_key: resolved_key,
    }
}
