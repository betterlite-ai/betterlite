use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::error::VeoOperationError;

// _build_vertex_video_usage_from_request_data: the duration, resolution and video count the request asked for
pub fn video_usage(
    request_data: Option<&Object>,
    settings: &VertexSettings,
) -> Result<Object, VeoOperationError> {
    let mut usage = Object::new();
    let Some(data) = request_data else {
        return Ok(usage);
    };
    if data.is_empty() {
        return Ok(usage);
    }
    let parameters = request_parameters(data)?;
    let duration = match parameters.get("durationSeconds") {
        Some(value) if value.is_truthy() => float_value(value),
        _ => Number::from(settings.default_video_duration_seconds).as_f64(),
    };
    if let Some(seconds) = duration {
        let value = Number::from_f64(seconds).map_or(Json::Null, Json::Number);
        usage.insert("duration_seconds".to_owned(), value);
    }
    if let Some(resolution) = parameters.get("resolution").and_then(resolution_text) {
        usage.insert("video_resolution".to_owned(), Json::Text(resolution));
    }
    if let Some(count) = video_count(&parameters) {
        usage.insert("video_count".to_owned(), Json::Number(Number::from(count)));
    }
    Ok(usage)
}

// request_data.get("parameters", {}): an absent or null block is the empty default, and any other non-object is LiteLLM's AttributeError
fn request_parameters(data: &Object) -> Result<Object, VeoOperationError> {
    match data.get("parameters") {
        None | Some(Json::Null) => Ok(Object::new()),
        Some(Json::Object(fields)) => Ok(fields.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(VeoOperationError::ParametersShape)
        }
    }
}

// float(duration): a number or a decimal string is read, and a value float() raises on omits the duration
fn float_value(value: &Json) -> Option<f64> {
    match value {
        Json::Number(number) => number.as_f64(),
        Json::Text(text) => text.trim().parse::<f64>().ok(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// str(res).strip().lower(): a string as itself, a number in decimal notation, and nothing else
fn resolution_text(value: &Json) -> Option<String> {
    let text = match value {
        Json::Text(text) => text.clone(),
        Json::Number(number) => number.to_string(),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => return None,
    };
    let normalized = text.trim().to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

// veo_video_count_from_parameters: a positive JSON integer, and a bool is never a count
fn video_count(parameters: &Object) -> Option<u64> {
    match parameters.get("sampleCount") {
        Some(Json::Number(number)) => number.as_u64().filter(|count| *count >= 1),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
