use std::collections::{BTreeMap, BTreeSet};

use betterlite_anthropic::anthropic_http_stream::completion::ParameterError;
use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_anthropic::feature_flags::beta::user_beta_headers;
use betterlite_anthropic::feature_flags::messages::{
    is_cache_control_set, is_file_id_used, is_mcp_server_used, is_pdf_used,
};
use betterlite_anthropic::feature_flags::tools::computer_tool_version;
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::headers::request::{
    HeaderFeature, HeaderFeatures, HeaderRequest, get_anthropic_headers,
};
use betterlite_azure::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use betterlite_azure::credentials::selection::AzureTokenError;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::anthropic_messages::environment::{ParamsKey, TokenSource};

const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const TOOLS_FIELD: &str = "tools";
const MCP_SERVERS_FIELD: &str = "mcp_servers";
const IS_VERTEX_REQUEST_FIELD: &str = "is_vertex_request";

// This matches the B564 input-shape precedence while the shared helper remains outside this task's scope.
fn explicit_key<'data>(
    params_key: ParamsKey<'data>,
    argument: Option<&'data SecretString>,
) -> Option<&'data SecretString> {
    let usable = argument.filter(|key| !key.expose_secret().is_empty());
    match params_key {
        ParamsKey::DictNull => None,
        ParamsKey::Dict(key) => Some(key),
        ParamsKey::Typed(Some(key)) if !key.expose_secret().is_empty() => Some(key),
        ParamsKey::DictAbsent | ParamsKey::Typed(_) => usable,
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error(transparent)]
    Token(#[from] AzureTokenError),
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error(transparent)]
    Parameter(#[from] ParameterError),
}

impl EnvironmentError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Token(source) => source.status(),
            Self::Features(source) => source.status_code(),
            Self::Parameter(_) => Some(400),
        }
    }
}

pub struct EnvironmentRequest<'data> {
    pub caller_headers: IndexMap<String, String>,
    pub params_key: ParamsKey<'data>,
    pub argument_key: Option<&'data SecretString>,
    pub configured_key: Option<&'data SecretString>,
    pub token: &'data TokenSource<'data>,
    pub messages: &'data [Json],
    pub optional_params: &'data IndexMap<String, Json>,
}

// LiteLLM builds the Anthropic headers from the call's key, then lets Azure auth and the caller win
pub fn validate_environment(
    request: &EnvironmentRequest<'_>,
) -> Result<AnthropicHeaders, EnvironmentError> {
    let features = detected_features(request)?;
    let betas = user_beta_headers(
        request
            .caller_headers
            .get(ANTHROPIC_BETA_HEADER)
            .map(String::as_str),
    );
    let generated = get_anthropic_headers(&HeaderRequest {
        api_key: request
            .argument_key
            .filter(|key| !key.expose_secret().is_empty()),
        auth_token: None,
        anthropic_version: None,
        api_base: None,
        use_bearer_for_custom_base: false,
        user_betas: betas.as_deref(),
        features: &features,
    });
    let azure = validate_azure_environment(EnvironmentHeadersRequest {
        headers: caller_headers(request),
        api_key: explicit_key(request.params_key, request.argument_key)
            .filter(|key| !key.expose_secret().is_empty())
            .or(request.configured_key),
        credentials: request.token.credentials,
        refresh_enabled: request.token.refresh_enabled,
        provider: request.token.provider.clone(),
        sources: request.token.sources,
    })?;
    let mut values = generated.values;
    values.extend(azure);
    values
        .entry(ANTHROPIC_VERSION_HEADER.to_owned())
        .or_insert_with(|| ANTHROPIC_VERSION.to_owned());
    Ok(AnthropicHeaders { values })
}

fn caller_headers(request: &EnvironmentRequest<'_>) -> BTreeMap<String, String> {
    request
        .caller_headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn detected_features(request: &EnvironmentRequest<'_>) -> Result<HeaderFeatures, EnvironmentError> {
    let messages: Vec<PromptJson> = request.messages.iter().map(prompt_json).collect();
    let tools = option_list(request.optional_params, TOOLS_FIELD)?;
    let mcp_servers = option_list(request.optional_params, MCP_SERVERS_FIELD)?;
    let mut features = BTreeSet::new();
    if is_cache_control_set(&messages) {
        features.insert(HeaderFeature::PromptCaching);
    }
    if is_pdf_used(&messages) {
        features.insert(HeaderFeature::Pdf);
    }
    if is_file_id_used(&messages)? {
        features.insert(HeaderFeature::FileId);
    }
    if is_mcp_server_used(mcp_servers.as_deref()) {
        features.insert(HeaderFeature::McpServer);
    }
    Ok(HeaderFeatures {
        features,
        computer_tool_used: tools
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find_map(computer_tool_version),
        is_vertex_request: matches!(
            request.optional_params.get(IS_VERTEX_REQUEST_FIELD),
            Some(Json::Bool(true))
        ),
        web_search_tool_used: false,
    })
}

fn option_list(
    options: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<Vec<PromptJson>>, EnvironmentError> {
    match options.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => Ok(Some(items.iter().map(prompt_json).collect())),
        Some(_) => Err(EnvironmentError::Parameter(ParameterError { field })),
    }
}

// LiteLLM passes the same values to the header checks and to the request transform
fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), prompt_json(item)))
                .collect(),
        ),
    }
}
