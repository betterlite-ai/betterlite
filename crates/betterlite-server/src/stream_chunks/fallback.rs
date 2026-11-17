use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::fallback_observations::error_info::fallback_errors_from_headers;
use betterlite_router::fallback_observations::headers::ATTEMPTED_FALLBACKS_HEADER;
use serde_json::Number;

const REQUESTED_MODEL_KEY: &str = "_litellm_client_requested_model";
const INCLUDE_FALLBACK_ERRORS_KEY: &str = "include_fallback_errors";
const MODEL_GROUP_HEADER: &str = "x-litellm-model-group";
const FALLBACK_EVENT_ID: &str = "litellm-fallback-metadata";
const FALLBACK_EVENT_OBJECT: &str = "chat.completion.chunk";

#[must_use]
pub fn client_requested_model(request_data: &JsonObject) -> String {
    let requested = request_data.get(REQUESTED_MODEL_KEY);
    if let Some(Json::Text(text)) = requested {
        return text.clone();
    }
    let model = request_data.get("model");
    if let Some(Json::Text(text)) = model {
        return text.clone();
    }
    String::new()
}

#[must_use]
pub fn is_positive_int_like(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null | Json::Bool(_)) => false,
        Some(Json::Number(number)) => positive_number(number),
        Some(Json::Text(text)) => positive_int_text(text),
        Some(Json::List(_) | Json::Object(_)) => false,
    }
}

fn positive_number(number: &Number) -> bool {
    if let Some(value) = number.as_i64() {
        return value.is_positive();
    }
    if let Some(value) = number.as_u64() {
        return value != 0;
    }
    number.as_f64().is_some_and(|value| value.trunc() > 0.0_f64)
}

fn positive_int_text(text: &str) -> bool {
    let trimmed = text.trim();
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    match digits_nonzero(digits) {
        Some(nonzero) => nonzero && !negative,
        None => false,
    }
}

// int("1_0") is 10, while "_1", "1_" and "1__0" raise ValueError
fn digits_nonzero(text: &str) -> Option<bool> {
    if text.is_empty() {
        return None;
    }
    let mut nonzero = false;
    for segment in text.split('_') {
        if segment.is_empty() {
            return None;
        }
        for byte in segment.bytes() {
            if !byte.is_ascii_digit() {
                return None;
            }
            nonzero |= byte != b'0';
        }
    }
    Some(nonzero)
}

#[must_use]
pub fn include_fallback_errors(expose: bool, request_data: &JsonObject) -> bool {
    if !expose {
        return false;
    }
    matches!(
        request_data.get(INCLUDE_FALLBACK_ERRORS_KEY),
        Some(Json::Bool(true))
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FallbackMeta {
    pub attempted: bool,
    pub model: Option<String>,
    pub errors: Vec<JsonObject>,
}

#[must_use]
pub fn fallback_metadata(hidden: &JsonObject) -> FallbackMeta {
    let Some(Json::Object(headers)) = hidden.get("additional_headers") else {
        return FallbackMeta::default();
    };
    if !is_positive_int_like(headers.get(ATTEMPTED_FALLBACKS_HEADER)) {
        return FallbackMeta::default();
    }
    FallbackMeta {
        attempted: true,
        model: headers.get(MODEL_GROUP_HEADER).and_then(model_group_text),
        errors: fallback_errors_from_headers(headers),
    }
}

fn model_group_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) if !text.is_empty() => Some(text.clone()),
        Json::Text(_)
        | Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}

#[must_use]
pub fn fallback_event_frame(model: Option<&str>, errors: &[JsonObject], created: i64) -> String {
    let mut payload = JsonObject::new();
    payload.insert("id".to_owned(), Json::Text(FALLBACK_EVENT_ID.to_owned()));
    payload.insert(
        "object".to_owned(),
        Json::Text(FALLBACK_EVENT_OBJECT.to_owned()),
    );
    payload.insert("created".to_owned(), Json::Number(Number::from(created)));
    payload.insert(
        "model".to_owned(),
        Json::Text(model.unwrap_or_default().to_owned()),
    );
    payload.insert("choices".to_owned(), Json::List(Vec::new()));
    let fallback_model = model.map_or(Json::Null, |text| Json::Text(text.to_owned()));
    let fallback_errors = Json::List(errors.iter().cloned().map(Json::Object).collect());
    let mut fallback = JsonObject::new();
    fallback.insert("fallback_model".to_owned(), fallback_model);
    fallback.insert("errors".to_owned(), fallback_errors);
    payload.insert("litellm_fallback".to_owned(), Json::Object(fallback));
    // Json always serializes; an impossible encoding failure must not lose the stream
    format!(
        "data: {}\n\n",
        serde_json::to_string(&Json::Object(payload)).unwrap_or_default()
    )
}
