use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

#[must_use]
pub fn later_chunk_model(chunks: &[JsonObject], assembled: Option<&Json>) -> bool {
    let Some(Json::Text(first)) = chunks.first().and_then(|chunk| chunk.get("model")) else {
        return false;
    };
    let Some(Json::Text(assembled_text)) = assembled else {
        return false;
    };
    !assembled_text.is_empty() && assembled_text != first
}

#[must_use]
pub fn client_named_model(request_data: &JsonObject, assembled: Option<&Json>) -> bool {
    let Some(stamped) = stamped_model(request_data) else {
        return false;
    };
    matches!(assembled, Some(Json::Text(name)) if name.as_str() == stamped)
}

pub fn restore_partial_model(
    partial: &mut JsonObject,
    chunks: &[JsonObject],
    wrapper_model: Option<&str>,
    request_data: &JsonObject,
) {
    let assembled = partial.get("model");
    if later_chunk_model(chunks, assembled) && !client_named_model(request_data, assembled) {
        return;
    }
    if let Some(model) = wrapper_model.filter(|model| !model.is_empty()) {
        partial.insert("model".to_owned(), Json::Text(model.to_owned()));
    }
}

fn stamped_model(request_data: &JsonObject) -> Option<&str> {
    text(request_data.get("_litellm_client_requested_model"))
        .or_else(|| text(request_data.get("model")))
}

fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
