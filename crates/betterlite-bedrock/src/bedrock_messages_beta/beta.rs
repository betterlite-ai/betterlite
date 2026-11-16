use std::collections::{BTreeMap, BTreeSet};
use std::slice;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::beta::{BetaRequest, beta_list};
use betterlite_anthropic::feature_flags::messages::{
    has_output_config, has_thinking_display_updates, has_tool_change, is_file_id_used,
    is_mcp_server_used,
};
use betterlite_anthropic::feature_flags::tools::{
    computer_tool_version, is_eager_input_streaming_used,
};
use betterlite_anthropic::feature_flags::{
    AnthropicFeatures, Feature, FeatureFlagsError, FeatureParams,
};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::anthropic_beta_filter::error::BetaFilterError;
use betterlite_core::api::anthropic_beta_filter::matrix_filter::filter_beta_tokens;
use betterlite_logging::analytics::python::{FormatError, repr};
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use super::context_management::filter_context_management;
use super::provider::{CUSTOM_LLM_PROVIDER, beta_headers_provider};
use super::tool_search::{
    TOOL_EXAMPLES_BETA, TOOL_SEARCH_BETA, adjust_tool_search_beta, beta_mode,
};
use crate::event_frames::beta::{BetaHeader, BetaValue, anthropic_beta_from_headers};

pub const DANGEROUS_TOOL_USE_BETA: &str = "dangerous-tool-use-2026-09-03";
pub const FINE_GRAINED_STREAMING_BETA: &str = "fine-grained-tool-streaming-2025-05-14";
pub const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
const DROPPED_USER_BETAS_WARNING: &str =
    "Bedrock Invoke: dropping unsupported anthropic-beta values from client headers";

pub struct BetaHeadersRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: &'request [Json],
    pub mcp_servers: &'request [Json],
    pub safeguards: Option<&'request Json>,
    pub params: &'request FeatureParams,
    pub headers: &'request BTreeMap<String, BetaHeader>,
    pub outgoing: &'request mut Object,
    pub clear_thinking_injected: bool,
    pub subclass_provider: Option<&'request str>,
}

#[derive(Debug, Error)]
pub enum BetaHeadersError {
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error(transparent)]
    Filter(#[from] BetaFilterError),
    #[error("cannot render the dropped beta header names")]
    Format(#[from] FormatError),
}

impl BetaHeadersError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Features(error) => error.status_code(),
            Self::Filter(_) | Self::Format(_) => None,
        }
    }
}

pub async fn beta_headers<Source, Transport>(
    request: BetaHeadersRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
    matrix: &BetaMatrix,
) -> Result<Vec<String>, BetaHeadersError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let BetaHeadersRequest {
        model,
        messages,
        tools,
        mcp_servers,
        safeguards,
        params,
        headers,
        outgoing,
        clear_thinking_injected,
        subclass_provider,
    } = request;
    let provider = beta_headers_provider(subclass_provider);
    let user_betas = user_beta_tokens(headers);
    let mut betas: BTreeSet<String> = user_betas.iter().cloned().collect();
    let detected = features(messages, tools, mcp_servers, outgoing)?;
    let generated = generated_betas(model, params, &detected, environment, catalogs).await;
    betas.extend(generated.into_iter().map(str::to_owned));
    add_extra_betas(tools, safeguards, clear_thinking_injected, &mut betas)?;
    filter_context_management(outgoing, &mut betas);
    adjust_tool_search_beta(model, beta_mode(tools), &mut betas, environment, catalogs).await;
    filtered_betas(betas, &user_betas, matrix, provider)
}

async fn generated_betas<Source, Transport>(
    model: &str,
    params: &FeatureParams,
    detected: &AnthropicFeatures,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> BTreeSet<&'static str>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    beta_list(
        &BetaRequest {
            model,
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            params,
            features: detected,
            prompt_caching_set: false,
        },
        environment,
        catalogs,
    )
    .await
}

fn filtered_betas(
    mut betas: BTreeSet<String>,
    user_betas: &BTreeSet<String>,
    matrix: &BetaMatrix,
    provider: &str,
) -> Result<Vec<String>, BetaHeadersError> {
    if betas.contains(TOOL_SEARCH_BETA) {
        betas.insert(TOOL_EXAMPLES_BETA.to_owned());
    }
    let all: Vec<String> = betas.into_iter().collect();
    let filtered = filter_beta_tokens(matrix, provider, &all)?;
    warn_dropped_user_betas(user_betas, matrix, provider)?;
    Ok(filtered.tokens)
}

fn add_extra_betas(
    tools: &[Json],
    safeguards: Option<&Json>,
    clear_thinking_injected: bool,
    betas: &mut BTreeSet<String>,
) -> Result<(), FeatureFlagsError> {
    if clear_thinking_injected {
        betas.insert(INTERLEAVED_THINKING_BETA.to_owned());
    }
    if is_eager_input_streaming_used(tools)? {
        betas.insert(FINE_GRAINED_STREAMING_BETA.to_owned());
    }
    if safeguards.is_some_and(|value| !matches!(value, Json::Null)) {
        betas.insert(DANGEROUS_TOOL_USE_BETA.to_owned());
    }
    Ok(())
}

fn features(
    messages: &[Json],
    tools: &[Json],
    mcp_servers: &[Json],
    outgoing: &Object,
) -> Result<AnthropicFeatures, FeatureFlagsError> {
    let mut flags = BTreeSet::new();
    if is_file_id_used(messages)? {
        flags.insert(Feature::FileId);
    }
    if is_mcp_server_used(Some(mcp_servers)) {
        flags.insert(Feature::McpServer);
    }
    let outgoing_messages = json_list(outgoing.get("messages"));
    if has_output_config(outgoing_messages) {
        flags.insert(Feature::MidConversationOutputConfig);
    }
    if has_thinking_display_updates(outgoing.get("thinking")) {
        flags.insert(Feature::ThinkingDisplayUpdates);
    }
    if has_tool_change(outgoing_messages) {
        flags.insert(Feature::MidConversationToolChange);
    }
    Ok(AnthropicFeatures {
        flags,
        computer_tool: first_computer_tool(tools),
    })
}

fn json_list(value: Option<&Json>) -> &[Json] {
    if let Some(Json::List(items)) = value {
        return items;
    }
    &[]
}

fn first_computer_tool(tools: &[Json]) -> Option<String> {
    for tool in tools {
        let version = computer_tool_version(tool);
        if version.is_some() {
            return version;
        }
    }
    None
}

fn user_beta_tokens(headers: &BTreeMap<String, BetaHeader>) -> BTreeSet<String> {
    let mut betas = BTreeSet::new();
    for value in anthropic_beta_from_headers(headers) {
        if let BetaValue::Text(text) = value {
            betas.insert(text);
        }
    }
    betas
}

fn warn_dropped_user_betas(
    user_betas: &BTreeSet<String>,
    matrix: &BetaMatrix,
    provider: &str,
) -> Result<(), BetaHeadersError> {
    let mut dropped = Vec::new();
    for beta in user_betas {
        let filtered = filter_beta_tokens(matrix, provider, slice::from_ref(beta))?;
        if filtered.tokens.is_empty() {
            dropped.push(beta.clone());
        }
    }
    if dropped.is_empty() {
        return Ok(());
    }
    let names = repr(&dropped)?;
    tracing::warn!(
        "{DROPPED_USER_BETAS_WARNING}: {}. Bedrock has no mapping entry for these; forwarding them would cause a 400.",
        names
    );
    Ok(())
}
