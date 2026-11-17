use std::collections::{BTreeMap, BTreeSet};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::beta::{BetaRequest, beta_list};
use betterlite_anthropic::feature_flags::messages::{
    has_output_config, has_thinking_display_updates, is_file_id_used, is_mcp_server_used,
};
use betterlite_anthropic::feature_flags::tools::{
    computer_tool_version, is_eager_input_streaming_used,
};
use betterlite_anthropic::feature_flags::{
    AnthropicFeatures, Feature, FeatureFlagsError, FeatureParams,
};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::beta_headers::ANTHROPIC_BETA_HEADER;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::anthropic_beta_filter::error::BetaFilterError;
use betterlite_core::api::anthropic_beta_filter::matrix_filter::filter_beta_tokens;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

use crate::bedrock_messages_beta::beta::FINE_GRAINED_STREAMING_BETA;
use crate::bedrock_messages_beta::tool_search::{adjust_tool_search_beta, beta_mode};
use crate::event_frames::beta::{BetaHeader, BetaValue, anthropic_beta_from_headers};

use super::CUSTOM_LLM_PROVIDER;
use super::wire;

pub const BETA_HEADER_FIELD: &str = "anthropic_beta";

#[derive(Debug, thiserror::Error)]
pub enum InvokeBetaError {
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error(transparent)]
    Filter(#[from] BetaFilterError),
    #[error("the request body does not round-trip as JSON")]
    Body(#[source] serde_json::Error),
}

impl InvokeBetaError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Features(error) => error.status_code(),
            Self::Filter(_) | Self::Body(_) => None,
        }
    }
}

pub struct InvokeBetaRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [ContractJson],
    pub optional_params: &'request IndexMap<String, ContractJson>,
    pub headers: &'request AnthropicHeaders,
    pub thinking: Option<&'request Json>,
}

// user betas pass through untouched; only the generated ones are filtered and transformed.
pub async fn compute_beta_headers<Source, Transport>(
    request: &InvokeBetaRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
    matrix: &BetaMatrix,
) -> Result<Vec<String>, InvokeBetaError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let user_betas = user_betas(request.headers);
    let tools = tools(request.optional_params)?;
    let messages: Vec<Json> = wire::convert(request.messages).map_err(InvokeBetaError::Body)?;
    let features = features(request, &messages, &tools)?;
    let mut betas = user_betas.clone();
    let params = FeatureParams {
        compaction: None,
        container: None,
        output_config: request.optional_params.get("output_config").cloned(),
        reasoning_effort: request.optional_params.get("reasoning_effort").cloned(),
    };
    let generated = beta_list(
        &BetaRequest {
            model: request.model,
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            params: &params,
            features: &features,
            prompt_caching_set: false,
        },
        environment,
        catalogs,
    )
    .await;
    betas.extend(generated.into_iter().map(str::to_owned));
    adjust_tool_search_beta(
        request.model,
        beta_mode(&tools),
        &mut betas,
        environment,
        catalogs,
    )
    .await;
    if is_eager_input_streaming_used(&tools)? {
        betas.insert(FINE_GRAINED_STREAMING_BETA.to_owned());
    }
    let auto_betas: Vec<String> = betas.difference(&user_betas).cloned().collect();
    let filtered = filter_beta_tokens(matrix, CUSTOM_LLM_PROVIDER, &auto_betas)?;
    let mut computed = user_betas;
    computed.extend(filtered.tokens);
    Ok(computed.into_iter().collect())
}

fn features(
    request: &InvokeBetaRequest<'_>,
    messages: &[Json],
    tools: &[Json],
) -> Result<AnthropicFeatures, InvokeBetaError> {
    let mut flags = BTreeSet::new();
    if is_file_id_used(messages)? {
        flags.insert(Feature::FileId);
    }
    let mcp_servers = mcp_servers(request.optional_params)?;
    if is_mcp_server_used(mcp_servers.as_deref()) {
        flags.insert(Feature::McpServer);
    }
    if has_output_config(messages) {
        flags.insert(Feature::MidConversationOutputConfig);
    }
    if has_thinking_display_updates(request.thinking) {
        flags.insert(Feature::ThinkingDisplayUpdates);
    }
    Ok(AnthropicFeatures {
        flags,
        computer_tool: first_computer_tool(tools),
    })
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

fn tools(optional_params: &IndexMap<String, ContractJson>) -> Result<Vec<Json>, InvokeBetaError> {
    let Some(ContractJson::List(items)) = optional_params.get("tools") else {
        return Ok(Vec::new());
    };
    wire::convert(items).map_err(InvokeBetaError::Body)
}

fn mcp_servers(
    optional_params: &IndexMap<String, ContractJson>,
) -> Result<Option<Vec<Json>>, InvokeBetaError> {
    let Some(ContractJson::List(items)) = optional_params.get("mcp_servers") else {
        return Ok(None);
    };
    let converted = wire::convert(items).map_err(InvokeBetaError::Body)?;
    Ok(Some(converted))
}

// The header value reaches B599 as the one text value a dict lookup would hand it.
fn user_betas(headers: &AnthropicHeaders) -> BTreeSet<String> {
    let mut user = BTreeSet::new();
    let Some(value) = headers.values.get(ANTHROPIC_BETA_HEADER) else {
        return user;
    };
    let mut supplied = BTreeMap::new();
    supplied.insert(
        ANTHROPIC_BETA_HEADER.to_owned(),
        BetaHeader::Text(value.clone()),
    );
    for beta in anthropic_beta_from_headers(&supplied) {
        if let BetaValue::Text(text) = beta {
            user.insert(text);
        }
    }
    user
}
