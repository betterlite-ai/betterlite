use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, translate_legacy_thinking,
};
use betterlite_anthropic::anthropic_capability_catalog::{EffortThresholds, OptionalParams};
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::claude_invoke::CUSTOM_LLM_PROVIDER;
use crate::claude_invoke::params::{MapParamsError, MapParamsRequest};
use crate::message_options::OptionsSources;
use crate::message_options::effort::clamp_adaptive_effort;
use crate::model_routes::{ParamMap, ParamValue};

const REASONING_EFFORT: &str = "reasoning_effort";

pub(super) fn thinking_param(
    non_default_params: &IndexMap<String, Json>,
) -> Result<Option<PromptJson>, MapParamsError> {
    let Some(value) = non_default_params.get("thinking") else {
        return Ok(None);
    };
    let converted = serde_json::from_value(serde_json::to_value(value)?)?;
    Ok(Some(converted))
}

// LiteLLM rewrites the caller's own map; the clamped copy is what the parent mapping reads here.
pub(super) async fn clamp_reasoning_effort<Source, Transport, Lookup>(
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

pub(super) async fn translate_thinking<Source, Transport, Lookup>(
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

pub(super) fn apply_display_updates(requested: bool, optional_params: &mut IndexMap<String, Json>) {
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
