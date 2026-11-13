use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_openai::base_helpers::thinking_request;
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::{CatalogSources, DropParams};

pub mod error;
pub mod max_tokens;
pub mod output_config;
pub mod thinking;
pub mod tools;

mod options;
mod tests;
mod value;

use error::ParamsError;

// LiteLLM's DEFAULT_MAX_TOKENS, the budget update adds to a thinking budget when no max_tokens came in
pub const DEFAULT_MAX_TOKENS: i64 = 4096;

pub struct ParamsRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub drop: DropParams,
    pub budgets: &'request Settings,
    pub default_max_tokens: i64,
    pub catalogs: &'request CatalogSources<'request>,
}

pub async fn map_openai_params<Source, Transport>(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let thinking_enabled = thinking_request(&value::base_params(non_default_params)?).thinking;
    for (param, value) in non_default_params {
        match param.as_str() {
            "response_format" => {
                tools::apply_response_format(
                    request,
                    optional_params,
                    value,
                    thinking_enabled,
                    environment,
                )
                .await?;
            }
            "thinking" => {
                thinking::apply_thinking(
                    request,
                    non_default_params,
                    optional_params,
                    value,
                    environment,
                )
                .await?;
            }
            "reasoning_effort" => {
                thinking::apply_reasoning_effort(request, optional_params, value, environment)
                    .await?;
            }
            _ => options::apply_plain(request, non_default_params, optional_params, param, value)?,
        }
    }
    thinking::update_max_tokens(request, non_default_params, optional_params)
}
