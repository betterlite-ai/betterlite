use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// LiteLLM prints the call id with %s, whose text for a missing value is None
pub(super) fn call_id_text(request_data: &JsonObject) -> &str {
    match request_data.get("litellm_call_id") {
        Some(Json::Text(call_id)) => call_id.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "None",
    }
}
