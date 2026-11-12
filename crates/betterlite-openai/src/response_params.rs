use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use serde_json::Value;

use crate::openai_base_config::{CatalogCapabilities, ModelFamily, model_family};
use crate::reasoning_models::{DropParams, ReasoningCapabilities, effort_resolves_to_none};

pub mod bundled;
pub mod capabilities;
pub mod decode;
pub mod error;
pub mod params;
mod tests;

use bundled::BundledReasoningModels;
use capabilities::{ModelInfo, ResponsesCapabilities};
use error::ResponseParamsError;
use params::{ResponsesOptions, ResponsesParams};

// ast-grep-ignore: no-long-names
pub const OPENAI_RESPONSES_API_MIN_MAX_OUTPUT_TOKENS: i64 = 16;

pub async fn map_openai_params<C>(
    capabilities: &mut C,
    options: &ResponsesOptions,
    drop_params: DropParams,
    bundled: &mut BundledReasoningModels,
) -> Result<ResponsesParams, ResponseParamsError>
where
    C: ResponsesCapabilities,
{
    let mut params = options.params.clone();
    enforce_min_max_output_tokens(&mut params);
    if options.provider == ProviderId::Openai
        && requests_reasoning_effort(params.reasoning.as_ref())
        && !supports_reasoning_param(capabilities, &options.model, bundled).await
    {
        if drop_params.global || drop_params.request {
            params.reasoning = None;
        } else {
            return Err(ResponseParamsError::UnsupportedReasoning {
                model: options.model.clone(),
            });
        }
    }
    if is_gpt_5_model(&options.model) {
        apply_gpt_sampling_gates(capabilities, options, drop_params, &mut params).await?;
    }
    Ok(params)
}

fn enforce_min_max_output_tokens(params: &mut ResponsesParams) {
    let Some(Some(Value::Number(number))) = &params.max_output_tokens else {
        return;
    };
    let Some(tokens) = number.as_i64() else {
        return;
    };
    // ast-grep-ignore: no-long-names
    let clamped = tokens.max(OPENAI_RESPONSES_API_MIN_MAX_OUTPUT_TOKENS);
    params.max_output_tokens = Some(Some(Value::from(clamped)));
}

fn requests_reasoning_effort(reasoning: Option<&Option<Value>>) -> bool {
    reasoning
        .and_then(Option::as_ref)
        .and_then(Value::as_object)
        .and_then(|fields| fields.get("effort"))
        .is_some_and(|effort| !effort.is_null())
}

async fn supports_reasoning_param<C>(
    capabilities: &mut C,
    model: &str,
    bundled: &mut BundledReasoningModels,
) -> bool
where
    C: ResponsesCapabilities,
{
    let lookup = model.rsplit('/').next().unwrap_or(model);
    match capabilities.model_info(lookup).await {
        ModelInfo::Unavailable => true,
        ModelInfo::Found {
            supports_reasoning: Some(declared),
            ..
        } => declared,
        ModelInfo::Found {
            key,
            supports_reasoning: None,
        } => bundled.get().contains(&key),
    }
}

fn is_gpt_5_model(model: &str) -> bool {
    let prefixed_elsewhere = match model.split_once('/') {
        Some((prefix, _)) => prefix != "openai",
        None => false,
    };
    let empty_catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    !prefixed_elsewhere && model_family(model, &empty_catalog) == ModelFamily::GptReasoning
}

async fn apply_gpt_sampling_gates<C>(
    capabilities: &mut C,
    options: &ResponsesOptions,
    drop_params: DropParams,
    params: &mut ResponsesParams,
) -> Result<(), ResponseParamsError>
where
    C: ResponsesCapabilities,
{
    let model = options.model.as_str();
    let effort = params
        .reasoning
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(Value::as_object)
        .and_then(|fields| fields.get("effort"));
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    let supports_none = capabilities
        .supports_factory(&request, NamedCapability::NoneReasoningEffort.key())
        .await;
    let resolves_none = supports_none && resolves_to_none(capabilities, model, effort).await;
    if let Some(rejected) = rejected_temperature(params, resolves_none) {
        return reject_sampling_param(params, model, drop_params, rejected);
    }
    if params.top_p.is_some() && !resolves_none {
        return reject_sampling_param(params, model, drop_params, Rejected::TopP);
    }
    Ok(())
}

enum Rejected {
    Temperature(Value),
    TopP,
}

fn rejected_temperature(params: &ResponsesParams, resolves_none: bool) -> Option<Rejected> {
    match params.temperature.as_ref() {
        Some(Some(temperature)) if !is_temperature_one(temperature) && !resolves_none => {
            Some(Rejected::Temperature(temperature.clone()))
        }
        Some(Some(_) | None) | None => None,
    }
}

fn reject_sampling_param(
    params: &mut ResponsesParams,
    model: &str,
    drop_params: DropParams,
    rejected: Rejected,
) -> Result<(), ResponseParamsError> {
    if drop_params.global || drop_params.request {
        match rejected {
            Rejected::Temperature(_) => params.temperature = None,
            Rejected::TopP => params.top_p = None,
        }
        return Ok(());
    }
    Err(match rejected {
        Rejected::Temperature(temperature) => ResponseParamsError::UnsupportedTemperature {
            model: model.to_owned(),
            temperature,
        },
        Rejected::TopP => ResponseParamsError::UnsupportedTopP {
            model: model.to_owned(),
        },
    })
}

async fn resolves_to_none<C>(capabilities: &mut C, model: &str, effort: Option<&Value>) -> bool
where
    C: ReasoningCapabilities,
{
    match effort {
        Some(Value::String(level)) => {
            effort_resolves_to_none(capabilities, model, Some(level)).await
        }
        Some(Value::Null) | None => effort_resolves_to_none(capabilities, model, None).await,
        Some(_) => false,
    }
}

fn is_temperature_one(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64() == Some(1.0),
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            false
        }
    }
}
