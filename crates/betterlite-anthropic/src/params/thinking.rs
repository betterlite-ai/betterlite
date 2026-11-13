use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_openai::base_helpers::add_thinking_max_tokens;
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::capability::supports_model_capability;
use crate::anthropic_capability_catalog::thinking::{ThinkingRequest, translate_legacy_thinking};
use crate::anthropic_capability_catalog::{EffortThresholds, OptionalParams};
use crate::capabilities::budget::cap_thinking_to_max_tokens;
use crate::capabilities::effort::EffortError;
use crate::capabilities::reasoning::{ReasoningRequest, map_reasoning_effort};
use crate::params::ParamsRequest;
use crate::params::error::ParamsError;
use crate::params::value;

pub const DROP_ADAPTIVE_THINKING_WARNING: &str = "Dropping adaptive `thinking` for model=%s: max_tokens is too small to fit the minimum thinking budget.";

// LiteLLM's REASONING_EFFORT_TO_OUTPUT_CONFIG_EFFORT keeps minimal as low and the rest as named
const fn effort_level(effort: &str) -> Option<&'static str> {
    match effort {
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}

async fn adaptive_model<Source, Transport>(
    request: &ParamsRequest<'_>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> bool
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    supports_model_capability(
        request.model,
        "supports_adaptive_thinking",
        request.provider,
        environment,
        request.catalogs,
    )
    .await
}

fn drop_adaptive(model: &str, optional_params: &mut IndexMap<String, Json>) {
    tracing::warn!(
        "{}",
        DROP_ADAPTIVE_THINKING_WARNING.replacen("%s", model, 1)
    );
    optional_params.shift_remove("thinking");
}

// Python's `max_completion_tokens or max_tokens`, then the int a budget can compare against
fn cap_tokens(non_default_params: &IndexMap<String, Json>) -> Result<Option<i64>, ParamsError> {
    let carries = |value: &Json| !match value {
        Json::Null | Json::Bool(false) => true,
        Json::Bool(true) => false,
        Json::Number(number) => number.as_f64() == Some(0.0_f64),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
    };
    let chosen = non_default_params
        .get("max_completion_tokens")
        .filter(|value| carries(value))
        .or_else(|| {
            non_default_params
                .get("max_tokens")
                .filter(|value| carries(value))
        });
    let Some(value) = chosen else {
        return Ok(None);
    };
    value::integer(value)
        .map(Some)
        .ok_or(ParamsError::NotNumber {
            field: "max_tokens",
        })
}

async fn cap_adaptive<Source, Transport>(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let limit = cap_tokens(non_default_params)?;
    let reasoning = ReasoningRequest {
        reasoning_effort: Some("medium"),
        model: request.model,
        custom_llm_provider: request.provider,
        llm_provider: request.provider,
        budgets: request.budgets,
    };
    let Some(legacy) = map_reasoning_effort(&reasoning, environment, request.catalogs).await?
    else {
        drop_adaptive(request.model, optional_params);
        return Ok(());
    };
    let Json::Object(fields) = &legacy else {
        drop_adaptive(request.model, optional_params);
        return Ok(());
    };
    match cap_thinking_to_max_tokens(fields, limit) {
        Some(capped) => {
            optional_params.insert("thinking".to_owned(), capped);
            Ok(())
        }
        None => {
            drop_adaptive(request.model, optional_params);
            Ok(())
        }
    }
}

async fn translate<Source, Transport>(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let mut params = OptionalParams {
        reasoning_effort: optional_params.get("reasoning_effort").cloned(),
        thinking: optional_params.get("thinking").cloned(),
        output_config: optional_params.get("output_config").cloned(),
    };
    let before_thinking = params.thinking.clone();
    let before_output = params.output_config.clone();
    let thinking_request = ThinkingRequest {
        model: request.model,
        custom_llm_provider: request.provider,
        thresholds: EffortThresholds {
            medium: request.budgets.medium,
            high: request.budgets.high,
            xhigh: request.budgets.xhigh,
        },
    };
    translate_legacy_thinking(
        &thinking_request,
        &mut params,
        environment,
        request.catalogs,
    )
    .await?;
    if params.thinking != before_thinking {
        match params.thinking {
            Some(thinking) => {
                optional_params.insert("thinking".to_owned(), thinking);
            }
            None => {
                optional_params.shift_remove("thinking");
            }
        }
    }
    if params.output_config != before_output
        && let Some(output_config) = params.output_config
    {
        optional_params.insert("output_config".to_owned(), output_config);
    }
    Ok(())
}

pub async fn apply_thinking<Source, Transport>(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let explicit = matches!(
        value::as_object(value).and_then(|fields| fields.get("type")),
        Some(Json::Text(kind)) if kind == "adaptive"
    );
    if explicit && !adaptive_model(request, environment).await {
        return cap_adaptive(request, non_default_params, optional_params, environment).await;
    }
    optional_params.insert("thinking".to_owned(), value.clone());
    translate(request, optional_params, environment).await
}

pub async fn apply_reasoning_effort<Source, Transport>(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let effort = match value {
        Json::Object(fields) => fields.get("effort"),
        other => Some(other),
    };
    let Some(Json::Text(effort)) = effort else {
        return Ok(());
    };
    let reasoning = ReasoningRequest {
        reasoning_effort: Some(effort),
        model: request.model,
        custom_llm_provider: request.provider,
        llm_provider: request.provider,
        budgets: request.budgets,
    };
    let Some(thinking) = map_reasoning_effort(&reasoning, environment, request.catalogs).await?
    else {
        optional_params.shift_remove("thinking");
        optional_params.shift_remove("output_config");
        return Ok(());
    };
    optional_params.insert("thinking".to_owned(), thinking);
    if !adaptive_model(request, environment).await {
        return Ok(());
    }
    let Some(level) = effort_level(effort) else {
        return Err(ParamsError::Effort(EffortError::InvalidReasoningEffort {
            value: effort.clone(),
            model: request.model.to_owned(),
            provider: request.provider.to_owned(),
        }));
    };
    let mut output_config = IndexMap::new();
    output_config.insert("effort".to_owned(), Json::Text(level.to_owned()));
    optional_params.insert("output_config".to_owned(), Json::Object(output_config));
    Ok(())
}

// The base policy's update_optional_params_with_thinking_tokens, run after every mapping
pub fn update_max_tokens(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
) -> Result<(), ParamsError> {
    let requested = value::base_params(non_default_params)?;
    let mut options = value::base_params(optional_params)?;
    add_thinking_max_tokens(&requested, &mut options, request.default_max_tokens)?;
    *optional_params = value::json_params(options)?;
    Ok(())
}
