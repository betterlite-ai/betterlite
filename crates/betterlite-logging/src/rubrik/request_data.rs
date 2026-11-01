use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[derive(Clone, Debug, Default)]
pub enum TrackedField<T> {
    #[default]
    Missing,
    Present(T),
}

#[derive(Clone, Debug, Default)]
pub struct RequestContextSource {
    pub messages: Option<Json>,
    pub model: Option<String>,
    pub tools: TrackedField<Json>,
    pub optional_params_tools: Option<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestDataPayload {
    pub messages: Option<Json>,
    pub model: Option<String>,
    pub tools: Option<Json>,
}

// Mirrors `{"messages": ..., "model": ..., "tools": ...}` when either source carries data, and the fully-empty mapping (no keys at all) when neither does.
#[must_use]
pub fn extract_request_data(
    call_details: Option<&RequestContextSource>,
    request_data: Option<&RequestContextSource>,
) -> Option<RequestDataPayload> {
    if call_details.is_none() && request_data.is_none() {
        return None;
    }
    let call_details_source = call_details.cloned().unwrap_or_default();
    let request_data_source = request_data.cloned().unwrap_or_default();

    // Use presence, not truthiness: an explicit empty tools list is the caller declaring the agent has no tools, forwarded as-is rather than falling through to optional_params.
    let tools = match request_data_source.tools {
        TrackedField::Present(value) => Some(value),
        TrackedField::Missing => call_details_source.optional_params_tools,
    };
    Some(RequestDataPayload {
        messages: call_details_source.messages,
        model: call_details_source.model,
        tools,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseSource {
    Absent,
    WithoutModel,
    WithModel(Option<String>),
}

#[must_use]
pub fn resolve_model(response: &ResponseSource, call_details_model: Option<&str>) -> String {
    match response {
        ResponseSource::WithModel(model) => {
            non_empty(model.as_deref()).unwrap_or("unknown").to_owned()
        }
        ResponseSource::Absent | ResponseSource::WithoutModel => {
            call_details_model.unwrap_or("unknown").to_owned()
        }
    }
}

// The id that joins a blocked request's moderation and failure logs: always litellm_call_id, present identically on both the guardrail and failure paths.
#[must_use]
pub fn correlation_id(
    call_details_call_id: Option<&str>,
    request_data_call_id: Option<&str>,
) -> Option<String> {
    let primary = non_empty(call_details_call_id);
    primary.map_or_else(
        || non_empty(request_data_call_id).map(str::to_owned),
        |id| Some(id.to_owned()),
    )
}

// Pins payload["id"] to litellm_call_id in place so this log shares its S3 filename id with the moderation and failure logs for the same request.
pub fn apply_correlation_id(payload: &mut IndexMap<String, Json>, source_call_id: Option<&str>) {
    if let Some(correlated) = non_empty(source_call_id) {
        payload.insert("id".to_owned(), Json::Text(correlated.to_owned()));
    }
}

const fn non_empty(text: Option<&str>) -> Option<&str> {
    match text {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}
