use std::collections::BTreeSet;

use betterlite_config::sections::anthropic_credentials::DEFAULT_API_BASE;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use crate::headers::AnthropicHeaders;
use crate::headers::environment::{HeaderLiteLlmParams, HeaderParams};
use crate::headers::request::{HeaderFeature, HeaderFeatures, HeaderRequest};

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn settings(api_key: Option<&str>, auth_token: Option<&str>) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(secret),
        auth_token: auth_token.map(secret),
        api_base: DEFAULT_API_BASE.to_owned(),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

pub(super) fn features(betas: &[HeaderFeature]) -> HeaderFeatures {
    HeaderFeatures {
        features: betas.iter().copied().collect(),
        computer_tool_used: None,
        is_vertex_request: false,
        web_search_tool_used: false,
    }
}

pub(super) fn request<'request>(
    api_key: Option<&'request SecretString>,
    auth_token: Option<&'request SecretString>,
    features: &'request HeaderFeatures,
) -> HeaderRequest<'request> {
    HeaderRequest {
        api_key,
        auth_token,
        anthropic_version: None,
        api_base: None,
        use_bearer_for_custom_base: false,
        user_betas: None,
        features,
    }
}

pub(super) fn params() -> HeaderParams {
    HeaderParams::default()
}

pub(super) fn params_thinking(thinking: &str) -> HeaderParams {
    HeaderParams {
        tools: None,
        mcp_servers: None,
        thinking: Some(json(thinking)),
        is_vertex_request: false,
        container: None,
    }
}

pub(super) fn params_tools(tools: &[&str]) -> HeaderParams {
    HeaderParams {
        tools: Some(tools.iter().copied().map(json).collect()),
        mcp_servers: None,
        thinking: None,
        is_vertex_request: false,
        container: None,
    }
}

pub(super) fn params_mcp(servers: &[&str]) -> HeaderParams {
    HeaderParams {
        tools: None,
        mcp_servers: Some(servers.iter().copied().map(json).collect()),
        thinking: None,
        is_vertex_request: false,
        container: None,
    }
}

pub(super) fn params_vertex(tools: &[&str]) -> HeaderParams {
    HeaderParams {
        tools: Some(tools.iter().copied().map(json).collect()),
        mcp_servers: None,
        thinking: None,
        is_vertex_request: true,
        container: None,
    }
}

pub(super) fn params_container(container: &str) -> HeaderParams {
    HeaderParams {
        tools: None,
        mcp_servers: None,
        thinking: None,
        is_vertex_request: false,
        container: Some(serde_json::from_str(container).unwrap()),
    }
}

pub(super) fn litellm_params() -> HeaderLiteLlmParams {
    HeaderLiteLlmParams::default()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn header<'headers>(
    headers: &'headers AnthropicHeaders,
    name: &str,
) -> Option<&'headers str> {
    headers.values.get(name).map(String::as_str)
}

pub(super) fn beta_set(headers: &AnthropicHeaders) -> BTreeSet<String> {
    headers
        .values
        .get("anthropic-beta")
        .map(|value| value.split(',').map(str::to_owned).collect())
        .unwrap_or_default()
}
