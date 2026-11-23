use betterlite_context::non_default_params::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::{DataResidency, infer_openai_data_residency};
use indexmap::IndexMap;

use crate::api::responses_provider_selection::ParameterMap;

use super::call::ResponsesCreateCall;
use super::convert::decision_json;
use super::preparation::Resolved;

// litellm_logging_obj.update_from_kwargs, with litellm_params built as the wrapper builds it
pub fn log_call(
    state: &mut LoggingState,
    call: &ResponsesCreateCall,
    resolved: &Resolved,
    filtered: &Params,
    is_async: bool,
    kwargs: &ParameterMap,
) -> Result<(), LoggingStateError> {
    let mut litellm_params: IndexMap<String, Json> = filtered
        .iter()
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect();
    litellm_params.insert(String::from("aresponses"), Json::Bool(is_async));
    litellm_params.insert(
        String::from("litellm_call_id"),
        named(kwargs, "litellm_call_id"),
    );
    litellm_params.insert(String::from("model_info"), named(kwargs, "model_info"));
    litellm_params.insert(
        String::from("data_residency"),
        residency_json(infer_openai_data_residency(
            Some(resolved.provider.as_str()),
            resolved.api_base.as_deref(),
        )),
    );
    // an explicit litellm_metadata wins over the named metadata even when it is null
    let metadata = if kwargs.contains_key("litellm_metadata") {
        named(kwargs, "litellm_metadata")
    } else {
        named(kwargs, "metadata")
    };
    litellm_params.insert(String::from("metadata"), metadata);
    litellm_params.insert(
        String::from("provider_affinity_header"),
        named(kwargs, "provider_affinity_header"),
    );
    let optional_params = filtered
        .iter()
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect();
    let additional_params = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(resolved.provider.clone()),
    )]);
    let locals = kwargs
        .iter()
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect();
    update_from_kwargs(
        state,
        &locals,
        Some(&litellm_params),
        CallUpdate {
            model: Some(resolved.model.clone()),
            user: call.user.clone(),
            optional_params,
            additional_params,
        },
    )
}

fn named(kwargs: &ParameterMap, name: &str) -> Json {
    kwargs.get(name).map_or(Json::Null, decision_json)
}

fn residency_json(residency: Option<DataResidency>) -> Json {
    match residency {
        Some(DataResidency::Eu) => Json::Text(String::from("eu")),
        Some(DataResidency::Us) => Json::Text(String::from("us")),
        None => Json::Null,
    }
}
