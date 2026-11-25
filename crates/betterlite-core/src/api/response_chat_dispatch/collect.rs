use std::error::Error;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::error::DispatchError;
use super::port::{BridgeResponse, ResponsePayload, ResponsesEventStream};
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_to_chat_stream::{DecodedEventLine, decode_event_line};

// _collect_response_from_stream: the drain, then the completed event's response beside the iterator's hidden params
pub async fn collect<Source>(
    mut events: Box<dyn ResponsesEventStream>,
    limits: &ApiLimits,
) -> Result<BridgeResponse, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let mut completed = Terminal::Pending;
    while let Some(event_line) = events.next_event().await {
        let line = event_line.map_err(|failure| DispatchError::Stream { failure })?;
        if let DecodedEventLine::Event(event) = decode_event_line(&line, limits)?
            && let Some(terminal) = terminal_response(&event)
        {
            completed = terminal;
        }
    }
    let stream_hidden = events.hidden_params().clone();
    let Terminal::Response(found) = completed else {
        return Err(DispatchError::MissingCompletedResponse);
    };
    let payload = serde_json::from_value::<Json>(found)?;
    let existing_hidden = existing_hidden(&payload);
    Ok(BridgeResponse {
        payload: ResponsePayload::Dict(payload),
        existing_hidden,
        stream_hidden,
    })
}

// _coerce_response_object: a Responses object stays, a dict is constructed, and anything else is the payload error
pub fn coerce<Source>(
    payload: ResponsePayload,
) -> Result<ResponsesApiResponse, DispatchError<Source>>
where
    Source: Error + 'static,
{
    match payload {
        ResponsePayload::Typed(response) => Ok(*response),
        ResponsePayload::Dict(Json::Object(fields)) => construct(fields),
        ResponsePayload::Dict(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        ) => Err(DispatchError::UnexpectedPayload),
    }
}

// ResponsesAPIResponse(**dict) with model_construct's tolerance: whatever fails validation is left out
fn construct<Source>(
    fields: IndexMap<String, Json>,
) -> Result<ResponsesApiResponse, DispatchError<Source>>
where
    Source: Error + 'static,
{
    let Value::Object(mut object) = serde_json::to_value(Json::Object(fields))? else {
        return Err(DispatchError::UnexpectedPayload);
    };
    // each pass drops a key or fills a default, so the bound only trips a repair that stops making progress
    for _ in 0..=object.len().saturating_add(missing_defaults().len()) {
        let error = match serde_path_to_error::deserialize::<_, ResponsesApiResponse>(
            Value::Object(object.clone()),
        ) {
            Ok(response) => return Ok(response),
            Err(error) => error,
        };
        if !drop_invalid(&mut object, &error) {
            return Err(DispatchError::Completion { source: error });
        }
    }
    Err(DispatchError::UnexpectedPayload)
}

fn drop_invalid(
    object: &mut Map<String, Value>,
    error: &serde_path_to_error::Error<serde_json::Error>,
) -> bool {
    let path = error.path().to_string();
    if let Some(first) = path.split('.').next().filter(|first| !first.is_empty()) {
        return object.remove(first).is_some();
    }
    let message = error.inner().to_string();
    for (field, default) in missing_defaults() {
        let needle = format!("missing field `{field}`");
        if message.contains(needle.as_str()) {
            object.insert(field.to_owned(), default);
            return true;
        }
    }
    false
}

fn missing_defaults() -> [(&'static str, Value); 3] {
    [
        ("id", Value::String(String::new())),
        ("created_at", Value::from(0_i64)),
        ("output", Value::Array(Vec::new())),
    ]
}

// The last terminal event decides the collected response, even when it carries none
enum Terminal {
    Pending,
    WithoutResponse,
    Response(Value),
}

fn terminal_response(event: &Value) -> Option<Terminal> {
    let Value::Object(fields) = event else {
        return None;
    };
    let Some(Value::String(kind)) = fields.get("type") else {
        return None;
    };
    if !matches!(
        kind.as_str(),
        "response.completed" | "response.incomplete" | "response.failed"
    ) {
        return None;
    }
    Some(match fields.get("response") {
        Some(Value::Null) | None => Terminal::WithoutResponse,
        Some(response) => Terminal::Response(response.clone()),
    })
}

fn existing_hidden(payload: &Json) -> Map<String, Value> {
    let Json::Object(fields) = payload else {
        return Map::new();
    };
    let Some(Json::Object(hidden)) = fields.get("_hidden_params") else {
        return Map::new();
    };
    hidden
        .iter()
        .filter_map(|(key, value)| Some((key.clone(), serde_json::to_value(value).ok()?)))
        .collect()
}

// existing keys win, and every stream key the response leaves absent fills in
#[must_use]
pub fn merged_hidden(
    existing: &Map<String, Value>,
    stream: &Map<String, Value>,
) -> Map<String, Value> {
    let mut merged = existing.clone();
    for (key, value) in stream {
        merged.entry(key.clone()).or_insert_with(|| value.clone());
    }
    merged
}
