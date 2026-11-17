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
    thinking: &ParamValue,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let native = is_deepseek_model(request.model, &get_bedrock_base_model(request.model));
    if native {
        tracing::debug!(
            "Dropping unsupported `thinking` param for Bedrock model={}; it reasons natively.",
            request.model
        );
    }
    if native || is_openai_gpt_reasoning_model(request.model) {
        return Ok(());
    }
    if is_explicit_adaptive(thinking)
        && !supports_model_capability(
            request.model,
            ADAPTIVE_CAPABILITY,
            BEDROCK_PROVIDER,
            environment,
            request.catalogs,
        )
        .await
    {
        if let Some(mapped) = apply_adaptive_fallback(request, environment).await? {
            optional_params.insert(THINKING_FIELD.to_owned(), mapped);
        }
        return Ok(());
    }
    optional_params.insert(THINKING_FIELD.to_owned(), thinking.clone());
    translate_for_adaptive_model(request, optional_params, environment).await
}

// A model without adaptive thinking still gets the medium legacy budget, capped by the request's limit.
async fn apply_adaptive_fallback<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Option<ParamValue>, ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let limit = cap_limit(request.non_default_params)?;
    let mapped = map_reasoning_effort(
        &ReasoningRequest {
            reasoning_effort: Some(MEDIUM_EFFORT),
            model: request.model,
            custom_llm_provider: BEDROCK_PROVIDER,
            llm_provider: CONVERSE_PROVIDER,
            budgets: request.budgets,
        },
        environment,
        request.catalogs,
    )
    .await?;
    let Some(capped) = mapped
        .as_ref()
        .and_then(as_object)
        .and_then(|fields| cap_thinking_to_max_tokens(fields, limit))
    else {
        tracing::warn!(
            "{}",
            DROP_ADAPTIVE_THINKING_WARNING.replacen("%s", request.model, 1)
        );
        return Ok(None);
    };
    Ok(Some(transcode(&capped)?))
}

// B688 turns a legacy budget into an effort only on models that dropped the legacy field.
async fn translate_for_adaptive_model<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let mut params = OptionalParams {
        reasoning_effort: optional_param_json(optional_params, REASONING_EFFORT_FIELD)?,
        thinking: optional_param_json(optional_params, THINKING_FIELD)?,
        output_config: optional_param_json(optional_params, OUTPUT_CONFIG_FIELD)?,
    };
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
    if let Some(thinking) = params.thinking {
        optional_params.insert(THINKING_FIELD.to_owned(), transcode(&thinking)?);
    }
    if let Some(output_config) = params.output_config {
        optional_params.insert(OUTPUT_CONFIG_FIELD.to_owned(), transcode(&output_config)?);
    }
    Ok(())
}

// The `reasoning_effort` branch: only a string reaches B600, after the R1 models get their debug log.
pub async fn apply_reasoning_effort<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    requested: &ParamValue,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let ParamValue::Text(effort) = requested else {
        return Ok(());
    };
    if rejects_reasoning_effort(request.model, &get_bedrock_base_model(request.model)) {
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
    .await
    .map_err(ConverseParamsError::Options)
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
    let chosen = match completion {
        Some(value) if wire::carries_value(value) => completion,
        _ => non_default_params.get(MAX_TOKENS_FIELD),
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
