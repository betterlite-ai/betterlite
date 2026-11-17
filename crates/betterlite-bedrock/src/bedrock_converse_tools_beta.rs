pub mod beta;
pub mod context_management;
pub mod convert;
pub mod error;
pub mod json;
pub mod partition;

mod tests;

use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json as PromptsJson;
use betterlite_prompts::bedrock_tools::capabilities::BedrockToolCapabilities;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::beta::{apply_anthropic_betas, initial_betas};
use crate::bedrock_converse_tools_beta::convert::convert_tools;
use crate::bedrock_converse_tools_beta::error::ToolsAndBetaError;
use crate::bedrock_converse_tools_beta::json::convert_all;
use crate::bedrock_converse_tools_beta::partition::partition_tools;
use crate::event_frames::beta::BetaHeader;

pub struct ToolsAndBetaRequest<'request> {
    pub tools: &'request [Json],
    pub model: &'request str,
    pub headers: Option<&'request BTreeMap<String, BetaHeader>>,
    pub params: &'request mut IndexMap<String, Json>,
    pub capabilities: BedrockToolCapabilities,
}

pub struct ToolsAndBeta {
    pub tools: Vec<Json>,
    pub betas: Vec<Json>,
    pub names: ToolNameMap,
}

pub async fn process_tools_and_beta<Source, Transport>(
    request: ToolsAndBetaRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<ToolsAndBeta, ToolsAndBetaError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let ToolsAndBetaRequest {
        tools,
        model,
        headers,
        params,
        capabilities,
    } = request;
    let partition = partition_tools(tools);
    let prompts: Vec<PromptsJson> = convert_all(&partition.filtered)?;
    // LiteLLM tests the base model for the anthropic family, not the caller's model id
    let anthropic_base = get_bedrock_base_model(model).starts_with("anthropic");
    let mut betas = initial_betas(headers, &prompts, anthropic_base)?;
    let mut converted = convert_tools(&partition.filtered, &prompts, model, capabilities, params)?;
    if let Some(beta) = converted.computer_beta {
        betas.push(Json::Text(beta.to_owned()));
    }
    converted.blocks.extend(partition.pre_formatted);
    if anthropic_base {
        apply_anthropic_betas(params, model, &mut betas, environment, catalogs).await;
    }
    Ok(ToolsAndBeta {
        tools: converted.blocks,
        betas,
        names: converted.names,
    })
}
