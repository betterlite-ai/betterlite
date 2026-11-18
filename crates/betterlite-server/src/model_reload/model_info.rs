use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::static_eligibility::environment::active_in_environment;

// model_info_as_mapping: a DB row's model_info column arrives as a mapping or as the JSON text it was stored as.
#[must_use]
pub fn model_info_as_mapping(model_info: &Json) -> Option<JsonObject> {
    match model_info {
        Json::Object(mapping) => Some(mapping.clone()),
        Json::Text(text) => parsed_mapping(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}

fn parsed_mapping(text: &str) -> Option<JsonObject> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(mapping)) => Some(mapping),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => None,
    }
}

// _expects_liveness_on_this_pod: a model_info the environment rule refuses still expects to serve, so its drop is reported.
#[must_use]
pub fn expects_liveness_on_this_pod(model_info: &Json, environment: Option<&str>) -> bool {
    model_info_as_mapping(model_info)
        .is_none_or(|mapping| active_in_environment(&mapping, environment).unwrap_or(true))
}
