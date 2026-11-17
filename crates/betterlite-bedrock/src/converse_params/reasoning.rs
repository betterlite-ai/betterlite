use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingRequest, translate_legacy_thinking,
};
use betterlite_anthropic::anthropic_capability_catalog::{EffortThresholds, OptionalParams};
use betterlite_anthropic::capabilities::budget::cap_thinking_to_max_tokens;
use betterlite_anthropic::capabilities::reasoning::{ReasoningRequest, map_reasoning_effort};
use betterlite_anthropic::params::error::ParamsError;
use betterlite_anthropic::params::thinking::DROP_ADAPTIVE_THINKING_WARNING;
use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::effort::normalize_opus_effort;
use crate::converse_options::family::{
    is_deepseek_model, is_openai_gpt_reasoning_model, rejects_reasoning_effort,
};
use crate::converse_options::reasoning::{ReasoningSources, handle_reasoning_effort};
use crate::converse_params::compose::ConverseParamsRequest;
use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::wire::{self, transcode};
use crate::model_routes::{ParamMap, ParamValue};

const ADAPTIVE_CAPABILITY: &str = "supports_adaptive_thinking";
const ADAPTIVE_TYPE: &str = "adaptive";
const BEDROCK_PROVIDER: &str = "bedrock";
const CONVERSE_PROVIDER: &str = "bedrock_converse";
const MAX_COMPLETION_TOKENS_FIELD: &str = "max_completion_tokens";
const MAX_TOKENS_FIELD: &str = "max_tokens";
const MEDIUM_EFFORT: &str = "medium";
const OUTPUT_CONFIG_FIELD: &str = "output_config";
const REASONING_EFFORT_FIELD: &str = "reasoning_effort";
const THINKING_FIELD: &str = "thinking";
const TYPE_FIELD: &str = "type";

// The `thinking` branch: DeepSeek and the GPT reasoning models never take the field, everyone else maps it.
pub async fn apply_thinking<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    value: &ParamValue,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let base_model = get_bedrock_base_model(request.model);
    if is_deepseek_model(request.model, &base_model) {
        tracing::debug!(
            "Dropping unsupported `thinking` param for Bedrock model={}; it reasons natively.",
            request.model
        );
        return Ok(());
    }
    if is_openai_gpt_reasoning_model(request.model) {
        return Ok(());
    }
    if is_explicit_adaptive(value)
        && !supports_model_capability(
            request.model,
            ADAPTIVE_CAPABILITY,
            BEDROCK_PROVIDER,
            environment,
            request.catalogs,
        )
        .await
    {
        // A model without adaptive thinking still gets the medium legacy budget, capped by the request's limit.
        let reasoning = ReasoningRequest {
            reasoning_effort: Some(MEDIUM_EFFORT),
            model: request.model,
            custom_llm_provider: BEDROCK_PROVIDER,
            llm_provider: CONVERSE_PROVIDER,
            budgets: request.budgets,
        };
        let limit = cap_limit(request.non_default_params)?;
        let legacy = map_reasoning_effort(&reasoning, environment, request.catalogs).await?;
        let capped = legacy
            .as_ref()
            .and_then(as_object)
            .and_then(|fields| cap_thinking_to_max_tokens(fields, limit));
        let Some(thinking) = capped else {
            tracing::warn!(
                "{}",
                DROP_ADAPTIVE_THINKING_WARNING.replacen("%s", request.model, 1)
            );
            return Ok(());
        };
        optional_params.insert(THINKING_FIELD.to_owned(), transcode(&thinking)?);
        return Ok(());
    }
    optional_params.insert(THINKING_FIELD.to_owned(), value.clone());
    // B688 turns a legacy budget into an effort only on models that dropped the legacy field.
    let mut params = OptionalParams {
        reasoning_effort: optional_param_json(optional_params, REASONING_EFFORT_FIELD)?,
        thinking: optional_param_json(optional_params, THINKING_FIELD)?,
        output_config: optional_param_json(optional_params, OUTPUT_CONFIG_FIELD)?,
    };
    let before_thinking = params.thinking.clone();
    let before_output = params.output_config.clone();
    let legacy_request = ThinkingRequest {
        model: request.model,
        custom_llm_provider: BEDROCK_PROVIDER,
        thresholds: EffortThresholds {
            medium: request.budgets.medium,
            high: request.budgets.high,
            xhigh: request.budgets.xhigh,
        },
    };
    translate_legacy_thinking(&legacy_request, &mut params, environment, request.catalogs).await?;
    if params.thinking != before_thinking {
        match params.thinking {
            Some(thinking) => {
                optional_params.insert(THINKING_FIELD.to_owned(), transcode(&thinking)?);
            }
            None => {
                optional_params.shift_remove(THINKING_FIELD);
            }
        }
    }
    if params.output_config != before_output
        && let Some(output_config) = params.output_config
    {
        optional_params.insert(OUTPUT_CONFIG_FIELD.to_owned(), transcode(&output_config)?);
    }
    Ok(())
}

// The `reasoning_effort` branch: only a string reaches B600, after the R1 models get their debug log.
pub async fn apply_reasoning_effort<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    value: &ParamValue,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let ParamValue::Text(effort) = value else {
        return Ok(());
    };
    let base_model = get_bedrock_base_model(request.model);
    if rejects_reasoning_effort(request.model, &base_model) {
        tracing::debug!(
            "Dropping unsupported `reasoning_effort` param for Bedrock model={}; it always reasons and rejects it.",
            request.model
        );
        return Ok(());
    }
    let sources = ReasoningSources {
        catalogs: request.catalogs,
        budgets: request.budgets,
        lookup: request.lookup,
        packaged: request.packaged,
    };
    handle_reasoning_effort(
        request.model,
        effort,
        optional_params,
        environment,
        &sources,
    )
    .await?;
    Ok(())
}

// The `output_config` branch copies the dict before B690 normalizes the effort ceiling.
pub async fn apply_output_config<Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    value: &ParamValue,
    optional_params: &mut ParamMap,
) -> Result<(), ConverseParamsError>
where
    Lookup: ModelInfoLookup,
{
    let mut output_config = value.clone();
    normalize_opus_effort(
        request.model,
        &mut output_config,
        request.lookup,
        request.packaged,
    )
    .await?;
    optional_params.insert(OUTPUT_CONFIG_FIELD.to_owned(), output_config);
    Ok(())
}

// Python's `max_completion_tokens or max_tokens`: a falsy value falls through, and null is absent.
fn cap_limit(non_default_params: &ParamMap) -> Result<Option<i64>, ConverseParamsError> {
    let completion = non_default_params.get(MAX_COMPLETION_TOKENS_FIELD);
    let chosen = if completion.is_some_and(wire::carries_value) {
        completion
    } else {
        non_default_params.get(MAX_TOKENS_FIELD)
    };
    match chosen {
        None | Some(ParamValue::Null) => Ok(None),
        Some(ParamValue::Bool(flag)) => Ok(Some(i64::from(*flag))),
        Some(ParamValue::Number(number)) => {
            number
                .as_i64()
                .map(Some)
                .ok_or(ConverseParamsError::Params(ParamsError::NotNumber {
                    field: MAX_TOKENS_FIELD,
                }))
        }
        Some(ParamValue::Text(_) | ParamValue::List(_) | ParamValue::Mapping(_)) => {
            Err(ConverseParamsError::Params(ParamsError::NotNumber {
                field: MAX_TOKENS_FIELD,
            }))
        }
    }
}

fn is_explicit_adaptive(value: &ParamValue) -> bool {
    matches!(value, ParamValue::Mapping(fields) if matches!(
        fields.get(TYPE_FIELD),
        Some(ParamValue::Text(kind)) if kind == ADAPTIVE_TYPE
    ))
}

const fn as_object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn optional_param_json(params: &ParamMap, key: &str) -> Result<Option<Json>, ConverseParamsError> {
    params
        .get(key)
        .map(transcode::<Json, ParamValue>)
        .transpose()
}
