use betterlite_anthropic::credentials::is_anthropic_oauth_key;

use crate::api::request_headers::ForwardedHeaders;
use crate::api::request_headers::cleaning::AUTHORIZATION;

// every provider that serves Anthropic's own API takes its version and beta headers
pub const ANTHROPIC_API_PROVIDERS: &str = "anthropic,bedrock,bedrock_mantle,vertex_ai";
// a caller's Anthropic OAuth token reaches only Anthropic itself
pub const ANTHROPIC_OAUTH_PROVIDERS: &str = "anthropic";
const ANTHROPIC_API_HEADERS: [&str; 2] = ["anthropic-version", "anthropic-beta"];

// ProviderSpecificHeader: extra headers scoped to a comma-separated provider list
#[derive(Debug, Clone)]
pub struct ProviderSpecificHeader {
    pub custom_llm_provider: &'static str,
    pub extra_headers: ForwardedHeaders,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderHeaders {
    pub scoped: Vec<ProviderSpecificHeader>,
    pub used_client_oauth_token: bool,
}

#[must_use]
pub fn provider_specific_headers(headers: &ForwardedHeaders) -> ProviderHeaders {
    let api_headers: ForwardedHeaders = ANTHROPIC_API_HEADERS
        .iter()
        .filter_map(|name| headers.get_key_value(*name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let oauth_headers: ForwardedHeaders = headers
        .iter()
        .filter(|(name, value)| {
            name.to_lowercase() == AUTHORIZATION && is_anthropic_oauth_key(Some(value.expose()))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let used_client_oauth_token = !oauth_headers.is_empty();
    let scoped = [
        (ANTHROPIC_API_PROVIDERS, api_headers),
        (ANTHROPIC_OAUTH_PROVIDERS, oauth_headers),
    ]
    .into_iter()
    .filter(|(_providers, extra_headers)| !extra_headers.is_empty())
    .map(
        |(custom_llm_provider, extra_headers)| ProviderSpecificHeader {
            custom_llm_provider,
            extra_headers,
        },
    )
    .collect();
    ProviderHeaders {
        scoped,
        used_client_oauth_token,
    }
}
