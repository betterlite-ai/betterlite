use super::call::ResponsesCreateCall;
use super::convert::decision_json;
use super::preparation::Resolved;
use crate::api::responses_provider_selection::ParameterMap;
use betterlite_context::non_default_params::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::{DataResidency, infer_openai_data_residency};
use indexmap::IndexMap;
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
        kwargs
            .get("litellm_call_id")
            .map_or(Json::Null, decision_json),
    );
    litellm_params.insert(
        String::from("model_info"),
        kwargs.get("model_info").map_or(Json::Null, decision_json),
    );
    litellm_params.insert(
        String::from("data_residency"),
        residency_json(infer_openai_data_residency(
            Some(resolved.provider.as_str()),
            resolved.api_base.as_deref(),
        )),
    );
    // an explicit litellm_metadata wins over the named metadata even when it is null
    let metadata = if kwargs.contains_key("litellm_metadata") {
        kwargs
            .get("litellm_metadata")
            .map_or(Json::Null, decision_json)
    } else {
        kwargs.get("metadata").map_or(Json::Null, decision_json)
    };
    litellm_params.insert(String::from("metadata"), metadata);
    litellm_params.insert(
        String::from("provider_affinity_header"),
        kwargs
            .get("provider_affinity_header")
            .map_or(Json::Null, decision_json),
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
fn residency_json(residency: Option<DataResidency>) -> Json {
    match residency {
        Some(DataResidency::Eu) => Json::Text(String::from("eu")),
        Some(DataResidency::Us) => Json::Text(String::from("us")),
        None => Json::Null,
    }
}
