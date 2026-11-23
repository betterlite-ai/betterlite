use super::convert::decision_json;
use super::preparation::{Prepared, Resolved};
use crate::api::responses_create_http::service::LogContext;
use betterlite_context::non_default_params::Params;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::{DataResidency, infer_openai_data_residency};
use indexmap::IndexMap;
pub fn log_call(
    log: &mut LogContext<'_, '_>,
    prep: &Prepared<'_>,
    resolved: &Resolved,
    filtered: &Params,
) -> Result<(), LoggingStateError> {
    let all = &prep.kwargs;
    let litellm_params = litellm_params(prep, resolved, filtered);
    let optional_params = json_map(filtered.iter());
    let additional_params = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(resolved.provider.clone()),
    )]);
    let locals = json_map(all.iter());
    update_from_kwargs(
        &mut *log.state,
        &locals,
        Some(&litellm_params),
        CallUpdate {
            model: Some(resolved.model.clone()),
            user: prep.call.user.clone(),
            optional_params,
            additional_params,
        },
    )
}
fn json_map<'value, I>(pairs: I) -> IndexMap<String, Json>
where
    I: Iterator<Item = (&'value String, &'value JsonValue)>,
{
    pairs
        .map(|(name, value)| (name.clone(), decision_json(value)))
        .collect()
}
fn litellm_params(
    prep: &Prepared<'_>,
    resolved: &Resolved,
    filtered: &Params,
) -> IndexMap<String, Json> {
    let all = &prep.kwargs;
    let metadata = if all.contains_key("litellm_metadata") {
        all.get("litellm_metadata")
    } else {
        all.get("metadata")
    };
    let mut params = json_map(filtered.iter());
    params.insert(String::from("aresponses"), Json::Bool(prep.is_async));
    params.insert(
        String::from("litellm_call_id"),
        all.get("litellm_call_id").map_or(Json::Null, decision_json),
    );
    params.insert(
        String::from("model_info"),
        all.get("model_info").map_or(Json::Null, decision_json),
    );
    let residency = infer_openai_data_residency(
        Some(resolved.provider.as_str()),
        resolved.api_base.as_deref(),
    );
    params.insert(String::from("data_residency"), residency_json(residency));
    params.insert(
        String::from("metadata"),
        metadata.map_or(Json::Null, decision_json),
    );
    params.insert(
        String::from("provider_affinity_header"),
        all.get("provider_affinity_header")
            .map_or(Json::Null, decision_json),
    );
    params
}
fn residency_json(residency: Option<DataResidency>) -> Json {
    match residency {
        Some(DataResidency::Eu) => Json::Text(String::from("eu")),
        Some(DataResidency::Us) => Json::Text(String::from("us")),
        None => Json::Null,
    }
}
