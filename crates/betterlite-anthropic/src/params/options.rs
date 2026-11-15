use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::sampling::{
    SamplingParam, SamplingRequest, apply_sampling_param,
};
use crate::capabilities::speed::{SpeedDecision, SpeedParam, SpeedRequest, maybe_drop_speed_param};
use crate::option_primitives::context_management::map_context_management;
use crate::option_primitives::map_stop_sequences;
use crate::option_primitives::user_id::UserIdPolicy;
use crate::params::error::ParamsError;
use crate::params::{ParamsRequest, max_tokens, tools, value};

// Every parameter LiteLLM's loop maps without an await, in the arms it gives them
pub(crate) fn apply(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    param: &str,
    value: &Json,
) -> Result<(), ParamsError> {
    match param {
        "max_tokens" => max_tokens::apply(optional_params, "max_tokens", value)?,
        "max_completion_tokens" => {
            max_tokens::apply(optional_params, "max_completion_tokens", value)?;
        }
        "tools" => tools::apply_tools(optional_params, value)?,
        "tool_choice" | "parallel_tool_calls" => {
            tools::apply_tool_choice(request, non_default_params, optional_params)?;
        }
        "stream" if matches!(value, Json::Bool(true)) => {
            optional_params.insert("stream".to_owned(), Json::Bool(true));
        }
        "stop" => apply_stop(request, optional_params, value)?,
        "user" => apply_user(optional_params, value)?,
        "context_management" => {
            if let Some(mapped) = map_context_management(value)? {
                optional_params.insert("context_management".to_owned(), Json::Object(mapped));
            }
        }
        "cache_control" if matches!(value, Json::Object(_)) => {
            optional_params.insert("cache_control".to_owned(), value.clone());
        }
        "temperature" | "top_p" => apply_sampling(request, optional_params, param, value)?,
        "speed" => apply_speed(request, optional_params, value)?,
        "web_search_options" => tools::apply_web_search(optional_params, value)?,
        "extra_headers" => {
            optional_params.insert("extra_headers".to_owned(), value.clone());
        }
        _ => {}
    }
    Ok(())
}

fn apply_stop(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
) -> Result<(), ParamsError> {
    let Some(sequences) = map_stop_sequences(value, request.drop.global)? else {
        return Ok(());
    };
    let mapped = sequences.into_iter().map(Json::Text).collect();
    optional_params.insert("stop_sequences".to_owned(), Json::List(mapped));
    Ok(())
}

fn apply_user(
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

fn apply_sampling(
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
    let mapped = apply_sampling_param(
        &SamplingRequest {
            model: request.model,
            param: sampling_param,
            value,
            drop: request.drop,
        },
        request.catalogs,
    )?;
    if let Some(kept) = mapped {
        optional_params.insert(sampling_param.key().to_owned(), kept);
    }
    Ok(())
}

fn apply_speed(
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
    if matches!(
        maybe_drop_speed_param(&speed, request.catalogs)?,
        SpeedDecision::Drop
    ) {
        optional_params.shift_remove("speed");
    }
    Ok(())
}
