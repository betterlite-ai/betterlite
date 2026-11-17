use betterlite_anthropic::anthropic_capability_catalog::sampling::{
    SamplingError, SamplingParam, SamplingRequest, apply_sampling_param,
};
use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::converse_options::family::requires_min_max_tokens;
use crate::converse_options::supported::supports_sampling_params;
use crate::converse_params::compose::ConverseParamsRequest;
use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::wire::transcode;
use crate::model_routes::{ParamMap, ParamValue};

const ANTHROPIC_PREFIX: &str = "anthropic";
const INFERENCE_CONFIG_FIELD: &str = "inferenceConfig";
const MIN_MAX_TOKENS: i64 = 16;
const NOVA_PREFIX: &str = "amazon.nova";
const STOP_FIELD: &str = "stopSequences";
const TOP_K_FIELD: &str = "top_k";
const TOP_K_CAMEL_FIELD: &str = "topK";
const TOP_P_FIELD: &str = "topP";

// LiteLLM's InferenceConfig takes topK, and an existing topK is overwritten by the snake_case value.
#[must_use]
pub fn transform_inference_params(mut inference_params: ParamMap) -> ParamMap {
    if let Some(value) = inference_params.shift_remove(TOP_K_FIELD) {
        inference_params.insert(TOP_K_CAMEL_FIELD.to_owned(), value);
    }
    inference_params
}

// The camel_case spelling wins, both present leaves top_k behind, and only Anthropic and Nova keep the value.
pub fn handle_top_k_value(
    model: &str,
    inference_params: &mut ParamMap,
    drop: DropParams,
    catalogs: &CatalogSources<'_>,
) -> Result<ParamMap, ConverseParamsError> {
    let chosen = inference_params
        .shift_remove(TOP_K_CAMEL_FIELD)
        .or_else(|| inference_params.shift_remove(TOP_K_FIELD));
    let Some(value) = chosen.filter(|value| !matches!(value, ParamValue::Null)) else {
        return Ok(ParamMap::new());
    };
    let base_model = get_bedrock_base_model(model);
    if base_model.starts_with(ANTHROPIC_PREFIX) {
        let encoded: Json = transcode(&value)?;
        let sampling = SamplingRequest {
            model,
            param: SamplingParam::TopK,
            value: &encoded,
            drop,
        };
        let Some(mapped) = apply_sampling_param(&sampling, catalogs)? else {
            return Ok(ParamMap::new());
        };
        return Ok(ParamMap::from([(
            TOP_K_FIELD.to_owned(),
            transcode(&mapped)?,
        )]));
    }
    if base_model.starts_with(NOVA_PREFIX) {
        let config = ParamMap::from([(TOP_K_CAMEL_FIELD.to_owned(), value)]);
        return Ok(ParamMap::from([(
            INFERENCE_CONFIG_FIELD.to_owned(),
            ParamValue::Mapping(config),
        )]));
    }
    Ok(ParamMap::new())
}

// Bedrock's OpenAI-compatible families reject a completion cap under 16; other values ride unchanged.
#[must_use]
pub fn max_tokens_value(model: &str, value: &ParamValue) -> ParamValue {
    if !requires_min_max_tokens(model) {
        return value.clone();
    }
    let below_minimum = match value {
        ParamValue::Bool(_) => true,
        ParamValue::Number(number) => number
            .as_i64()
            .is_some_and(|tokens| tokens < MIN_MAX_TOKENS),
        ParamValue::Null | ParamValue::Text(_) | ParamValue::List(_) | ParamValue::Mapping(_) => {
            false
        }
    };
    if below_minimum {
        return ParamValue::Number(Number::from(MIN_MAX_TOKENS));
    }
    value.clone()
}

// An empty string is the one stop value Converse rejects outright, so the field stays absent for it.
pub fn apply_stop(value: &ParamValue, optional_params: &mut ParamMap) {
    let sequences = match value {
        ParamValue::Text(text) if text.is_empty() => return,
        ParamValue::Text(text) => ParamValue::List(vec![ParamValue::Text(text.clone())]),
        ParamValue::Null
        | ParamValue::Bool(_)
        | ParamValue::Number(_)
        | ParamValue::List(_)
        | ParamValue::Mapping(_) => value.clone(),
    };
    optional_params.insert(STOP_FIELD.to_owned(), sequences);
}

// Anthropic models run the sampling gate, everyone else gets the flag or LiteLLM's 400.
pub fn apply_sampling<Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    optional_params: &mut ParamMap,
    param: &str,
    value: &ParamValue,
) -> Result<(), ConverseParamsError> {
    let sampling = if param == "temperature" {
        SamplingParam::Temperature
    } else {
        SamplingParam::TopP
    };
    let output_key = if param == "top_p" {
        TOP_P_FIELD
    } else {
        "temperature"
    };
    let encoded: Json = transcode(value)?;
    if get_bedrock_base_model(request.model).starts_with(ANTHROPIC_PREFIX) {
        let sampling_request = SamplingRequest {
            model: request.model,
            param: sampling,
            value: &encoded,
            drop: request.drop,
        };
        if let Some(kept) = apply_sampling_param(&sampling_request, request.catalogs)? {
            optional_params.insert(output_key.to_owned(), transcode(&kept)?);
        }
        return Ok(());
    }
    if supports_sampling_params(request.model, request.catalogs) {
        optional_params.insert(output_key.to_owned(), value.clone());
        return Ok(());
    }
    if request.drop.global || request.drop.call {
        return Ok(());
    }
    Err(ConverseParamsError::Sampling(SamplingError::Unsupported {
        model: request.model.to_owned(),
        param: sampling.key(),
        value: serde_json::to_string(&encoded).map_err(ConverseParamsError::Encode)?,
        hint: hint_for(sampling),
    }))
}

const fn hint_for(param: SamplingParam) -> &'static str {
    match param {
        SamplingParam::Temperature => "Only temperature=1 is supported. ",
        SamplingParam::TopP | SamplingParam::TopK => "",
    }
}
