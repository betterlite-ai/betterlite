use betterlite_core::api::request_destination::RequestData;
use betterlite_models::decisions::Json;

mod tests;

// LiteLLM str()s whatever it finds, so a numeric id names the same session bucket a text id would
fn session_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn metadata_session_id(data: &RequestData, bucket: &str) -> Option<String> {
    let Json::Object(fields) = data.get(bucket)? else {
        return None;
    };
    fields.get("session_id").and_then(session_text)
}

// LiteLLM's _get_session_id_from_data: litellm_session_id, then metadata.session_id, then litellm_metadata.session_id
#[must_use]
pub fn session_id_from_data(data: &RequestData) -> Option<String> {
    data.get("litellm_session_id")
        .and_then(session_text)
        .or_else(|| metadata_session_id(data, "metadata"))
        .or_else(|| metadata_session_id(data, "litellm_metadata"))
}
