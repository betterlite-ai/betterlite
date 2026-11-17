use std::collections::BTreeSet;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::beta::{BetaRequest, beta_list, tool_search_beta_header};
use betterlite_anthropic::feature_flags::messages::{is_file_id_used, is_mcp_server_used};
use betterlite_anthropic::feature_flags::tools::{computer_tool_version, is_tool_search_used};
use betterlite_anthropic::feature_flags::{
    AnthropicFeatures, Feature, FeatureFlagsError, FeatureParams,
};
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::request::beta_headers::{
    BetaHeaderError, COMPACT_BETA, CONTEXT_MANAGEMENT_BETA,
};
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use thiserror::Error;

use super::PROVIDER;
use super::json::{prompt_json, prompt_list};

const COMPACT_EDIT: &str = "compact_20260112";
const EDITS: &str = "edits";

#[derive(Debug, Error)]
pub enum BetaError {
    #[error(transparent)]
    Features(#[from] FeatureFlagsError),
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error(transparent)]
    Context(#[from] BetaHeaderError),
    #[error("context_management must be an object")]
    ContextManagementNotObject,
    #[error("extra_headers must be an object")]
    ExtraHeadersNotObject,
    #[error("extra_headers.anthropic-beta entries must be strings")]
    BetaEntryNotText,
}

impl BetaError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Features(error) => error.status_code(),
            Self::Params(error) => error.status_code(),
            Self::Context(error) => error.status_code(),
            Self::ContextManagementNotObject
            | Self::ExtraHeadersNotObject
            | Self::BetaEntryNotText => Some(400),
        }
    }
}

// The subclass's own set: B568's automatic betas, the Vertex tool-search beta, its context edits and
// the caller's anthropic-beta extras. The parent's header step is skipped, so this is the whole set.
pub async fn compose<Source, Transport>(
    model: &str,
    params: &IndexMap<String, Json>,
    messages: &[Json],
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<BTreeSet<String>, BetaError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let tools = prompt_list(params, "tools")?;
    let mcp_servers = prompt_list(params, "mcp_servers")?;
    let prompt_messages: Vec<PromptJson> = messages.iter().map(prompt_json).collect();
    let mut flags = BTreeSet::new();
    if is_file_id_used(&prompt_messages)? {
        flags.insert(Feature::FileId);
    }
    if is_mcp_server_used(Some(mcp_servers.as_slice())) {
        flags.insert(Feature::McpServer);
    }
    let features = AnthropicFeatures {
        flags,
        computer_tool: tools.iter().find_map(computer_tool_version),
    };
    let feature_params = FeatureParams {
        compaction: params.get("compaction").cloned(),
        container: params.get("container").cloned(),
        output_config: params.get("output_config").cloned(),
        reasoning_effort: params.get("reasoning_effort").cloned(),
    };
    let mut betas: BTreeSet<String> = beta_list(
        &BetaRequest {
            model,
            custom_llm_provider: PROVIDER,
            params: &feature_params,
            features: &features,
            prompt_caching_set: false,
        },
        environment,
        catalogs,
    )
    .await
    .into_iter()
    .map(str::to_owned)
    .collect();
    if is_tool_search_used(&tools) {
        betas.insert(tool_search_beta_header(PROVIDER).to_owned());
    }
    if let Some(context_management) = params.get("context_management") {
        add_context_management(&mut betas, context_management)?;
    }
    if let Some(extra_headers) = params.get("extra_headers") {
        merge_extra_headers(&mut betas, extra_headers)?;
    }
    Ok(betas)
}

// Only compact_20260112 selects the compact beta; every other edit type, "compaction" included,
// selects the context-management beta. The parent classifier folds "compaction" into compact.
fn add_context_management(
    betas: &mut BTreeSet<String>,
    context_management: &Json,
) -> Result<(), BetaError> {
    let edits = match context_management {
        Json::Null => return Ok(()),
        Json::Object(fields) => match fields.get(EDITS) {
            None | Some(Json::Null) => return Ok(()),
            Some(Json::List(edits)) => edits.as_slice(),
            Some(_) => return Err(BetaError::Context(BetaHeaderError::EditsNotList)),
        },
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(BetaError::ContextManagementNotObject);
        }
    };
    let mut compact = false;
    let mut managed = false;
    for (index, edit) in edits.iter().enumerate() {
        let Json::Object(fields) = edit else {
            return Err(BetaError::Context(BetaHeaderError::EditNotObject { index }));
        };
        if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == COMPACT_EDIT) {
            compact = true;
        } else {
            managed = true;
        }
    }
    if compact {
        betas.insert(COMPACT_BETA.to_owned());
    }
    if managed {
        betas.insert(CONTEXT_MANAGEMENT_BETA.to_owned());
    }
    Ok(())
}

fn merge_extra_headers(
    betas: &mut BTreeSet<String>,
    extra_headers: &Json,
) -> Result<(), BetaError> {
    let fields = match extra_headers {
        Json::Null => return Ok(()),
        Json::Object(fields) => fields,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(BetaError::ExtraHeadersNotObject);
        }
    };
    let value = match fields.get("anthropic-beta") {
        None | Some(Json::Null) => return Ok(()),
        Some(value) => value,
    };
    match value {
        Json::Text(text) => {
            for beta in text.split(',') {
                let trimmed = beta.trim();
                if !trimmed.is_empty() {
                    betas.insert(trimmed.to_owned());
                }
            }
            Ok(())
        }
        Json::List(entries) => {
            for entry in entries {
                let Json::Text(beta) = entry else {
                    return Err(BetaError::BetaEntryNotText);
                };
                betas.insert(beta.clone());
            }
            Ok(())
        }
        Json::Bool(_) | Json::Number(_) | Json::Object(_) => Ok(()),
    }
}
