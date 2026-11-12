use std::collections::BTreeSet;

use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::anthropic_capability_catalog::CatalogSources;
use crate::anthropic_capability_catalog::OptionalParams;
use crate::anthropic_capability_catalog::thinking::is_effort_used;

use super::{AnthropicFeatures, Feature, FeatureParams};

pub const EFFORT_BETA: &str = "effort-2025-11-24";
pub const FILES_API_BETA: &str = "files-api-2025-04-14";
pub const FILE_CODE_EXECUTION_BETA: &str = "code-execution-2025-05-22";
pub const MCP_CLIENT_BETA: &str = "mcp-client-2025-04-04";
pub const OUTPUT_CONFIG_BETA: &str = "mid-conversation-output-config-2026-07-01";
pub const THINKING_DISPLAY_BETA: &str = "thinking-display-updates-2026-08-18";
pub const TOOL_CHANGES_BETA: &str = "mid-conversation-tool-changes-2026-07-01";
const COMPUTER_2025_01_24: &str = "computer-use-2025-01-24";
const COMPUTER_2024_10_22: &str = "computer-use-2024-10-22";
const TOOL_SEARCH_VERTEX: &str = "tool-search-tool-2025-10-19";
const TOOL_SEARCH_ANTHROPIC: &str = "advanced-tool-use-2025-11-20";

pub struct BetaRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub params: Option<&'request FeatureParams>,
    pub features: &'request AnthropicFeatures,
    pub prompt_caching_set: bool,
}

#[must_use]
pub fn computer_tool_beta_header(version: &str) -> &'static str {
    match version {
        "computer_20250124" => COMPUTER_2025_01_24,
        _ => COMPUTER_2024_10_22,
    }
}

#[must_use]
pub fn tool_search_beta_header(custom_llm_provider: &str) -> &'static str {
    match custom_llm_provider {
        "vertex_ai" | "vertex_ai_beta" | "bedrock" => TOOL_SEARCH_VERTEX,
        _ => TOOL_SEARCH_ANTHROPIC,
    }
}

#[must_use]
pub fn user_beta_headers(header: Option<&str>) -> Option<Vec<String>> {
    if let Some(text) = header {
        return Some(text.split(',').map(str::to_owned).collect());
    }
    None
}

#[must_use]
pub async fn beta_list<Source, Transport>(
    request: &BetaRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> BTreeSet<&'static str>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let mut betas = feature_betas(request.features);
    let params = request.params.map(|params| OptionalParams {
        reasoning_effort: params.reasoning_effort.clone(),
        thinking: None,
        output_config: params.output_config.clone(),
    });
    if is_effort_used(
        params.as_ref(),
        request.model,
        request.custom_llm_provider,
        environment,
        catalogs,
    )
    .await
    {
        betas.insert(EFFORT_BETA);
    }
    betas
}

fn feature_betas(features: &AnthropicFeatures) -> BTreeSet<&'static str> {
    let mut betas = BTreeSet::new();
    if let Some(version) = features
        .computer_tool
        .as_deref()
        .filter(|candidate| !candidate.is_empty())
    {
        betas.insert(computer_tool_beta_header(version));
    }
    if features.flags.contains(&Feature::FileId) {
        betas.insert(FILES_API_BETA);
        betas.insert(FILE_CODE_EXECUTION_BETA);
    }
    if features.flags.contains(&Feature::McpServer) {
        betas.insert(MCP_CLIENT_BETA);
    }
    if features
        .flags
        .contains(&Feature::MidConversationOutputConfig)
    {
        betas.insert(OUTPUT_CONFIG_BETA);
    }
    if features.flags.contains(&Feature::ThinkingDisplayUpdates) {
        betas.insert(THINKING_DISPLAY_BETA);
    }
    if features.flags.contains(&Feature::MidConversationToolChange) {
        betas.insert(TOOL_CHANGES_BETA);
    }
    betas
}
