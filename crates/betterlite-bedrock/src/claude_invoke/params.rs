use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::anthropic_capability_catalog::forced_tool::forced_tool_use_unsupported;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingError, ThinkingRequest, translate_legacy_thinking,
};
use betterlite_anthropic::anthropic_capability_catalog::{
    CatalogSources, DropParams, EffortThresholds, OptionalParams,
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
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::message_options::MessageOptionsError;
use crate::message_options::OptionsSources;
use crate::message_options::effort::clamp_adaptive_effort;
use crate::model_routes::{ParamMap, ParamValue};

use super::{CUSTOM_LLM_PROVIDER, RESPONSE_FORMAT_STUB_MODEL};

const NATIVE_STRUCTURED_OUTPUT: &str = "supports_native_structured_output";
const REASONING_EFFORT: &str = "reasoning_effort";
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

pub async fn map_openai_params<Source, Transport, Lookup>(
    request: &MapParamsRequest<'_, Lookup>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), MapParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let original_model = request.model;
    let requested_thinking = thinking_param(non_default_params)?;
    let display_updates = has_thinking_display_updates(requested_thinking.as_ref());
    let has_response_format = non_default_params.contains_key(RESPONSE_FORMAT);
    let mut mapped_model = original_model;
    if has_response_format
        && !supports_model_capability(
            original_model,
            NATIVE_STRUCTURED_OUTPUT,
            CUSTOM_LLM_PROVIDER,
            environment,
            request.catalogs,
        )
        .await
    {
        // The stub model hides the original from the parent's own native-schema probe.
        mapped_model = RESPONSE_FORMAT_STUB_MODEL;
    }
    let mut clamped_params = non_default_params.clone();
    clamp_reasoning_effort(original_model, &mut clamped_params, request, environment).await?;
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
    translate_thinking(original_model, optional_params, request, environment).await?;
    apply_display_updates(display_updates, optional_params);
    remove_forced_tool_choice(
        has_response_format,
        original_model,
        optional_params,
        request.catalogs,
    );
    Ok(())
}

fn thinking_param(
    non_default_params: &IndexMap<String, Json>,
) -> Result<Option<PromptJson>, MapParamsError> {
    let Some(value) = non_default_params.get("thinking") else {
        return Ok(None);
    };
    let converted = serde_json::from_value(serde_json::to_value(value)?)?;
    Ok(Some(converted))
}

// LiteLLM rewrites the caller's own map; the clamped copy is what the parent mapping reads here.
async fn clamp_reasoning_effort<Source, Transport, Lookup>(
    model: &str,
    params: &mut IndexMap<String, Json>,
    request: &MapParamsRequest<'_, Lookup>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), MapParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let Some(Json::Text(effort)) = params.get(REASONING_EFFORT) else {
        return Ok(());
    };
    let mut clamped = ParamMap::new();
    clamped.insert(
        REASONING_EFFORT.to_owned(),
        ParamValue::Text(effort.clone()),
    );
    let mut sources = OptionsSources {
        environment: &mut *environment,
        catalogs: request.catalogs,
        budgets: request.budgets,
    };
    clamp_adaptive_effort(
        model,
        &mut clamped,
        &mut sources,
        request.lookup,
        request.packaged,
    )
    .await?;
    if let Some(ParamValue::Text(effort)) = clamped.get(REASONING_EFFORT) {
        params.insert(REASONING_EFFORT.to_owned(), Json::Text(effort.clone()));
    }
    Ok(())
}

async fn translate_thinking<Source, Transport, Lookup>(
    model: &str,
    optional_params: &mut IndexMap<String, Json>,
    request: &MapParamsRequest<'_, Lookup>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), MapParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let thinking_request = ThinkingRequest {
        model,
        custom_llm_provider: CUSTOM_LLM_PROVIDER,
        thresholds: EffortThresholds {
            medium: request.budgets.medium,
            high: request.budgets.high,
            xhigh: request.budgets.xhigh,
        },
    };
    let mut params = OptionalParams {
        reasoning_effort: optional_params.get(REASONING_EFFORT).cloned(),
        thinking: optional_params.get("thinking").cloned(),
        output_config: optional_params.get("output_config").cloned(),
    };
    let before = params.clone();
    translate_legacy_thinking(
        &thinking_request,
        &mut params,
        environment,
        request.catalogs,
    )
    .await?;
    write_back(
        optional_params,
        "thinking",
        &before.thinking,
        params.thinking,
    );
    write_back(
        optional_params,
        "output_config",
        &before.output_config,
        params.output_config,
    );
    Ok(())
}

// B568 changes only the entries it rewrote, so an untouched map keeps the caller's order.
fn write_back(
    optional_params: &mut IndexMap<String, Json>,
    key: &str,
    before: &Option<Json>,
    after: Option<Json>,
) {
    if before == &after {
        return;
    }
    match after {
        Some(value) => {
            optional_params.insert(key.to_owned(), value);
        }
        None => {
            optional_params.shift_remove(key);
        }
    }
}

fn apply_display_updates(requested: bool, optional_params: &mut IndexMap<String, Json>) {
    if !requested {
        return;
    }
    let Some(Json::Object(thinking)) = optional_params.get("thinking") else {
        return;
    };
    let adaptive = matches!(thinking.get("type"), Some(Json::Text(kind)) if kind == "adaptive");
    if !adaptive {
        return;
    }
    let mut rewritten = IndexMap::new();
    rewritten.insert("type".to_owned(), Json::Text("adaptive".to_owned()));
    rewritten.insert("display".to_owned(), Json::Text("updates".to_owned()));
    optional_params.insert("thinking".to_owned(), Json::Object(rewritten));
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
