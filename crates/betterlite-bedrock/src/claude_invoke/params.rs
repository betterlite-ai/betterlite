use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::anthropic_capability_catalog::forced_tool::forced_tool_use_unsupported;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::anthropic_capability_catalog::{
    CatalogSources, DropParams, EffortThresholds,
};
use betterlite_anthropic::capabilities::supported_params::get_supported_openai_params as anthropic_supported_params;
use betterlite_anthropic::feature_flags::messages::has_thinking_display_updates;
use betterlite_anthropic::params::ParamsRequest;
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::map_openai_params as anthropic_map_openai_params;
use betterlite_anthropic::params::tools::RESPONSE_FORMAT_TOOL_NAME;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::message_options::MessageOptionsError;

use super::{CUSTOM_LLM_PROVIDER, RESPONSE_FORMAT_STUB_MODEL};

mod thinking;

const NATIVE_STRUCTURED_OUTPUT: &str = "supports_native_structured_output";
const RESPONSE_FORMAT: &str = "response_format";
const TOOL_CHOICE: &str = "tool_choice";

#[derive(Debug, thiserror::Error)]
pub enum MapParamsError {
    #[error(transparent)]
    Params(#[from] ParamsError),
    #[error(transparent)]
    Options(#[from] MessageOptionsError),
    #[error(transparent)]
    Thinking(#[from] ThinkingError),
    #[error("the optional parameters do not round-trip as JSON")]
    Body(#[from] serde_json::Error),
}

impl MapParamsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Params(error) => error.status_code(),
            Self::Options(_) | Self::Thinking(_) | Self::Body(_) => None,
        }
    }
}

pub struct MapParamsRequest<'request, Lookup> {
    pub model: &'request str,
    pub drop: DropParams,
    pub budgets: &'request Settings,
    pub default_max_tokens: i64,
    pub catalogs: &'request CatalogSources<'request>,
    pub packaged: &'request ModelCatalog,
    pub lookup: &'request Lookup,
}

pub async fn get_supported_openai_params<Source, Transport>(
    model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Vec<&'static str>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    // LiteLLM's override hands the parent class its own custom_llm_provider, which is bedrock here.
    anthropic_supported_params(model, Some(CUSTOM_LLM_PROVIDER), environment, catalogs).await
}

pub async fn map_openai_params<'environment, Source, Transport, Lookup>(
    request: &MapParamsRequest<'environment, Lookup>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'environment, Source, Transport>,
) -> Result<(), MapParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let original_model = request.model;
    let requested_thinking = thinking::thinking_param(non_default_params)?;
    let display_updates = has_thinking_display_updates(requested_thinking.as_ref());
    let has_response_format = non_default_params.contains_key(RESPONSE_FORMAT);
    let mapped_model = stub_model(
        original_model,
        has_response_format,
        request.catalogs,
        environment,
    )
    .await;
    let clamped_params =
        thinking::clamp_reasoning_effort(request, non_default_params.clone(), environment).await?;
    let params_request = ParamsRequest {
        model: mapped_model,
        provider: CUSTOM_LLM_PROVIDER,
        drop: request.drop,
        budgets: request.budgets,
        default_max_tokens: request.default_max_tokens,
        catalogs: request.catalogs,
    };
    anthropic_map_openai_params(
        &params_request,
        &clamped_params,
        optional_params,
        environment,
    )
    .await?;
    translate_requested_thinking(request, optional_params, environment).await?;
    thinking::apply_display_updates(display_updates, optional_params);
    remove_forced_tool_choice(
        has_response_format,
        original_model,
        optional_params,
        request.catalogs,
    );
    Ok(())
}

async fn translate_requested_thinking<'environment, Source, Transport, Lookup>(
    request: &MapParamsRequest<'environment, Lookup>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'environment, Source, Transport>,
) -> Result<(), MapParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    thinking::translate_thinking(
        request.model,
        EffortThresholds {
            medium: request.budgets.medium,
            high: request.budgets.high,
            xhigh: request.budgets.xhigh,
        },
        optional_params,
        environment,
        request.catalogs,
    )
    .await
}

// The stub model hides the original from the parent's own native-schema probe.
async fn stub_model<'model, Source, Transport>(
    model: &'model str,
    has_response_format: bool,
    catalogs: &CatalogSources<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> &'model str
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if !has_response_format {
        return model;
    }
    let native = supports_model_capability(
        model,
        NATIVE_STRUCTURED_OUTPUT,
        CUSTOM_LLM_PROVIDER,
        environment,
        catalogs,
    )
    .await;
    if native {
        return model;
    }
    RESPONSE_FORMAT_STUB_MODEL
}

// The stub model hides the original model from the parent's forced-tool-use backstop.
fn remove_forced_tool_choice(
    has_response_format: bool,
    model: &str,
    optional_params: &mut IndexMap<String, Json>,
    catalogs: &CatalogSources<'_>,
) {
    if !has_response_format || !forced_tool_use_unsupported(model, catalogs) {
        return;
    }
    let Some(Json::Object(choice)) = optional_params.get(TOOL_CHOICE) else {
        return;
    };
    let named = matches!(
        choice.get("name"),
        Some(Json::Text(name)) if name == RESPONSE_FORMAT_TOOL_NAME
    );
    if named {
        optional_params.shift_remove(TOOL_CHOICE);
    }
}
