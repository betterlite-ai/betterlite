use crate::otel::model::utils::coercion::JsonValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct TraceControls {
    pub name: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub tags: Vec<String>,
}

#[must_use]
pub fn caller_trace_controls(kwargs: &JsonValue) -> TraceControls {
    let Some(params) = kwargs
        .as_object()
        .and_then(|fields| fields.get("litellm_params"))
        .and_then(JsonValue::as_object)
    else {
        return TraceControls::default();
    };
    let first = params.get("metadata").and_then(JsonValue::as_object);
    let second = params
        .get("litellm_metadata")
        .and_then(JsonValue::as_object);
    let headers = params
        .get("proxy_server_request")
        .and_then(JsonValue::as_object)
        .and_then(|request| request.get("headers"))
        .and_then(JsonValue::as_object);
    TraceControls {
        name: scalar(headers, first, second, "langfuse_trace_name", "trace_name"),
        user_id: scalar(
            headers,
            first,
            second,
            "langfuse_trace_user_id",
            "trace_user_id",
        ),
        session_id: scalar(headers, first, second, "langfuse_session_id", "session_id"),
        tags: trace_tags(first, second),
    }
}

fn scalar(
    headers: Option<&BTreeMap<String, JsonValue>>,
    first: Option<&BTreeMap<String, JsonValue>>,
    second: Option<&BTreeMap<String, JsonValue>>,
    header: &str,
    body_key: &str,
) -> Option<String> {
    let header_value = headers
        .and_then(|values| values.get(header))
        .and_then(nonempty_string);
    let first_value = first
        .and_then(|body| body.get(body_key))
        .and_then(nonempty_string);
    let second_value = second
        .and_then(|body| body.get(body_key))
        .and_then(nonempty_string);
    header_value.or(first_value).or(second_value)
}

fn nonempty_string(value: &JsonValue) -> Option<String> {
    match value.as_str() {
        Some(text) if !text.is_empty() => Some(text.to_owned()),
        Some(_) | None => None,
    }
}

fn trace_tags(
    first: Option<&BTreeMap<String, JsonValue>>,
    second: Option<&BTreeMap<String, JsonValue>>,
) -> Vec<String> {
    [first, second]
        .into_iter()
        .flatten()
        .find_map(|body| filtered_tags(body.get("tags")?))
        .unwrap_or_default()
}

fn filtered_tags(value: &JsonValue) -> Option<Vec<String>> {
    let tags: Vec<String> = value
        .as_array()?
        .iter()
        .filter_map(JsonValue::as_str)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();
    (!tags.is_empty()).then_some(tags)
}

mod tests;
