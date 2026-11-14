use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_params;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::ResponsesCapabilities;
use betterlite_openai::response_params::error::ResponseParamsError;
use betterlite_openai::response_params::params::ResponsesOptions;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::mantle_responses::config::{CUSTOM_LLM_PROVIDER, MantleResponsesConfig};
use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::repr::{quoted_string, repr_json};
use crate::mantle_responses::tools::filter_unsupported_tools;

const SUPPORTED_SERVICE_TIERS: [&str; 2] = ["auto", "default"];
const SUPPORTED_SERVICE_TIERS_REPR: &str = "['auto', 'default']";
const SUPPORTED_SUMMARIES: [&str; 1] = ["auto"];
const SUPPORTED_SUMMARIES_REPR: &str = "['auto']";

pub async fn map_openai_params<C>(
    config: &MantleResponsesConfig,
    capabilities: &mut C,
    options: &ResponsesOptions,
    drop_params: DropParams,
    bundled: &mut BundledReasoningModels,
) -> Result<Object, MantleResponsesError>
where
    C: ResponsesCapabilities,
{
    let lookup = model_map_lookup_name(&options.model);
    let parent = ResponsesOptions {
        provider: CUSTOM_LLM_PROVIDER,
        model: lookup,
        params: options.params.clone(),
    };
    // The catalog lookup decides the gpt-5 gate, while LiteLLM's rejection text still names the caller's model.
    let mapped =
        match response_params::map_openai_params(capabilities, &parent, drop_params, bundled).await
        {
            Ok(mapped) => mapped,
            Err(error) => {
                return Err(MantleResponsesError::Params(restore_original_model(
                    error,
                    &options.model,
                )));
            }
        };
    let raw = serde_json::to_value(&mapped).map_err(MantleResponsesError::Encode)?;
    let params: Object = serde_json::from_value(raw).map_err(MantleResponsesError::Encode)?;
    let tiered = drop_unsupported_tier(&params, drop_params)?;
    let summarized = drop_unsupported_summary(config, &tiered, &options.model, drop_params)?;
    filter_mapped_tools(summarized)
}

#[must_use]
pub fn model_map_lookup_name(model: &str) -> String {
    let segment = model.rsplit('/').next().unwrap_or(model);
    let stripped = segment.strip_prefix("openai.").unwrap_or(segment);
    stripped.to_owned()
}

fn restore_original_model(error: ResponseParamsError, model: &str) -> ResponseParamsError {
    match error {
        ResponseParamsError::UnsupportedTemperature { temperature, .. } => {
            ResponseParamsError::UnsupportedTemperature {
                model: model.to_owned(),
                temperature,
            }
        }
        ResponseParamsError::UnsupportedTopP { .. } => ResponseParamsError::UnsupportedTopP {
            model: model.to_owned(),
        },
        ResponseParamsError::UnsupportedReasoning { .. } => {
            ResponseParamsError::UnsupportedReasoning {
                model: model.to_owned(),
            }
        }
    }
}

pub(crate) fn drop_unsupported_tier(
    params: &Object,
    drop_params: DropParams,
) -> Result<Object, MantleResponsesError> {
    let Some(tier) = params.get("service_tier") else {
        return Ok(params.clone());
    };
    let supported =
        matches!(tier, Json::Text(name) if SUPPORTED_SERVICE_TIERS.contains(&name.as_str()));
    if supported || matches!(tier, Json::Null) {
        return Ok(params.clone());
    }
    if matches!(tier, Json::List(_) | Json::Object(_)) {
        return Err(MantleResponsesError::InvalidParams {
            message: format!(
                "service_tier={} cannot be checked against the Bedrock Mantle allowlist because only strings \
                 are comparable there",
                repr_json(tier)
            ),
        });
    }
    if !drop_params.global && !drop_params.request {
        return Err(MantleResponsesError::UnsupportedParams {
            message: format!(
                "bedrock_mantle does not support service_tier={}; the Bedrock Mantle \
                 Responses API only accepts 'auto' or 'default'. Set `drop_params: true` (litellm_settings \
                 or this deployment's litellm_params) to have LiteLLM drop it, or remove service_tier from \
                 the client (Codex CLI sends it when a speed tier is set in ~/.codex/config.toml).",
                repr_json(tier)
            ),
        });
    }
    tracing::warn!(
        "Bedrock Mantle Responses API: dropping unsupported service_tier {} (supported: {}).",
        repr_json(tier),
        SUPPORTED_SERVICE_TIERS_REPR
    );
    let mut dropped = params.clone();
    dropped.shift_remove("service_tier");
    Ok(dropped)
}

pub(crate) fn drop_unsupported_summary(
    config: &MantleResponsesConfig,
    params: &Object,
    model: &str,
    drop_params: DropParams,
) -> Result<Object, MantleResponsesError> {
    if !config.use_openai_path {
        return Ok(params.clone());
    }
    let Some(Json::Object(reasoning)) = params.get("reasoning") else {
        return Ok(params.clone());
    };
    let Some(summary) = reasoning.get("summary") else {
        return Ok(params.clone());
    };
    let supported =
        matches!(summary, Json::Text(name) if SUPPORTED_SUMMARIES.contains(&name.as_str()));
    if supported || matches!(summary, Json::Null) {
        return Ok(params.clone());
    }
    if !drop_params.global && !drop_params.request {
        return Err(MantleResponsesError::UnsupportedParams {
            message: format!(
                "bedrock_mantle does not support reasoning.summary={} for {}; the Bedrock Mantle \
                 OpenAI Responses path only accepts 'auto'. Set `drop_params: true` (litellm_settings or this \
                 deployment's litellm_params) to have LiteLLM drop it, or set `model_reasoning_summary = \"auto\"` \
                 in the client (Codex CLI: ~/.codex/config.toml).",
                repr_json(summary),
                quoted_string(model)
            ),
        });
    }
    tracing::warn!(
        "Bedrock Mantle Responses API: dropping unsupported reasoning.summary {} (supported: {}).",
        repr_json(summary),
        SUPPORTED_SUMMARIES_REPR
    );
    let mut stripped = Object::new();
    for (key, value) in reasoning {
        if key.as_str() != "summary" {
            stripped.insert(key.clone(), value.clone());
        }
    }
    let mut updated = params.clone();
    if stripped.is_empty() {
        updated.shift_remove("reasoning");
    } else {
        updated.insert("reasoning".to_owned(), Json::Object(stripped));
    }
    Ok(updated)
}

fn filter_mapped_tools(params: Object) -> Result<Object, MantleResponsesError> {
    let Some(raw_tools) = params.get("tools").cloned() else {
        return Ok(params);
    };
    if !raw_tools.is_truthy() {
        return Ok(params);
    }
    let listed = match raw_tools {
        Json::List(entries) => entries,
        single => vec![single],
    };
    let filtered = filter_unsupported_tools(&listed)?;
    let mut updated = params;
    if filtered.is_empty() {
        updated.shift_remove("tools");
    } else {
        updated.insert("tools".to_owned(), Json::List(filtered));
    }
    Ok(updated)
}
