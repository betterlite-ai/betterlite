use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::sampling::{
    SamplingParam, SamplingRequest, apply_sampling_param,
};
use crate::capabilities::speed::{SpeedDecision, SpeedParam, SpeedRequest, maybe_drop_speed_param};
use crate::option_primitives::context_management::map_context_management;
use crate::option_primitives::map_stop_sequences;
use crate::option_primitives::user_id::UserIdPolicy;
use crate::params::ParamsRequest;
use crate::params::error::ParamsError;
use crate::params::value;

// Only a literal true streams; any other value, false included, stays unmapped
pub fn apply_stream(optional_params: &mut IndexMap<String, Json>, value: &Json) {
    if matches!(value, Json::Bool(true)) {
        optional_params.insert("stream".to_owned(), Json::Bool(true));
    }
}

pub fn apply_stop(
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
    global_drop: bool,
) -> Result<(), ParamsError> {
    if !matches!(value, Json::Text(_) | Json::List(_)) {
        return Ok(());
    }
    match map_stop_sequences(value, global_drop)? {
        Some(sequences) => {
            let mapped = sequences.into_iter().map(Json::Text).collect();
            optional_params.insert("stop_sequences".to_owned(), Json::List(mapped));
            Ok(())
        }
        None => Ok(()),
    }
}

pub fn apply_sampling(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    param: &str,
    value: &Json,
) -> Result<(), ParamsError> {
    let sampling_param = if param == "temperature" {
        SamplingParam::Temperature
    } else {
        SamplingParam::TopP
    };
    let kept = apply_sampling_param(
        &SamplingRequest {
            model: request.model,
            param: sampling_param,
            value,
            drop: request.drop,
        },
        request.catalogs,
    )?;
    if let Some(kept) = kept {
        optional_params.insert(sampling_param.key().to_owned(), kept);
    }
    Ok(())
}

// An invalid, non-text or email-shaped user id is ignored rather than sent
pub fn apply_user(
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
) -> Result<(), ParamsError> {
    let Some(user_id) = value::as_str(value) else {
        return Ok(());
    };
    if !UserIdPolicy::compile()?.accepts(user_id) {
        return Ok(());
    }
    let mut metadata = IndexMap::new();
    metadata.insert("user_id".to_owned(), Json::Text(user_id.to_owned()));
    optional_params.insert("metadata".to_owned(), Json::Object(metadata));
    Ok(())
}

pub fn apply_context_management(
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
) -> Result<(), ParamsError> {
    if !matches!(value, Json::List(_) | Json::Object(_)) {
        return Ok(());
    }
    if let Some(mapped) = map_context_management(value)? {
        optional_params.insert("context_management".to_owned(), Json::Object(mapped));
    }
    Ok(())
}

pub fn apply_speed(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
) -> Result<(), ParamsError> {
    if value::as_str(value).is_none() {
        return Ok(());
    }
    optional_params.insert("speed".to_owned(), value.clone());
    let speed = SpeedRequest {
        model: request.model,
        speed: SpeedParam::Present(value),
        drop: request.drop,
        custom_llm_provider: Some(request.provider),
    };
    match maybe_drop_speed_param(&speed, request.catalogs)? {
        SpeedDecision::Keep => Ok(()),
        SpeedDecision::Drop => {
            optional_params.shift_remove("speed");
            Ok(())
        }
    }
}

pub fn apply_cache_control(optional_params: &mut IndexMap<String, Json>, value: &Json) {
    if matches!(value, Json::Object(_)) {
        optional_params.insert("cache_control".to_owned(), value.clone());
    }
}
