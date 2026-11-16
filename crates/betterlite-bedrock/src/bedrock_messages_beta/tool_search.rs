use std::collections::BTreeSet;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::tools::{
    is_tool_search_used, uses_input_examples, uses_programmatic_calls,
};
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json;

use crate::bedrock_model_capabilities::tool_search::supports_tool_search;

pub const ADVANCED_TOOL_USE_BETA: &str = "advanced-tool-use-2025-11-20";
pub const TOOL_SEARCH_BETA: &str = "tool-search-tool-2025-10-19";
pub const TOOL_EXAMPLES_BETA: &str = "tool-examples-2025-10-29";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSearchBeta {
    Adjust,
    Unchanged,
}

// LiteLLM adjusts the beta set only for tool search without programmatic calls or input examples.
#[must_use]
pub fn beta_mode(tools: &[Json]) -> ToolSearchBeta {
    let used = is_tool_search_used(tools);
    let shared = uses_programmatic_calls(tools) || uses_input_examples(tools);
    if used && !shared {
        ToolSearchBeta::Adjust
    } else {
        ToolSearchBeta::Unchanged
    }
}

pub async fn adjust_tool_search_beta<Source, Transport>(
    model: &str,
    mode: ToolSearchBeta,
    betas: &mut BTreeSet<String>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if mode == ToolSearchBeta::Unchanged {
        return;
    }
    betas.remove(ADVANCED_TOOL_USE_BETA);
    if supports_tool_search(model, environment, catalogs).await {
        betas.insert(TOOL_SEARCH_BETA.to_owned());
    }
}
