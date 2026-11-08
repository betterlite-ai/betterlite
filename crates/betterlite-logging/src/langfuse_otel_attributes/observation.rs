use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::Span;

use crate::langfuse_projections::steering::truthiness;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps, safe_json_loads};

use super::keys::LangfuseSpanAttributes;
use super::wire::{array, field, object};

// litellm.integrations.langfuse.langfuse_otel.LangfuseOtelLogger._set_observation_output
pub(crate) fn set_observation_output<S>(span: &mut S, response: Option<&Json>)
where
    S: Span,
{
    let Some(body) = response else {
        return;
    };
    let Some(fields) = object(body) else {
        return;
    };
    if fields.is_empty() {
        return;
    }
    if let Some(choices) = field(body, "choices")
        .and_then(array)
        .filter(|choices| !choices.is_empty())
        && let Some(choice) = choices.first()
    {
        set_choice_output(span, fields, choice);
    }
    if let Some(output) = field(body, "output")
        .and_then(array)
        .filter(|items| !items.is_empty())
    {
        set_output_items(span, output);
    }
}

fn set_choice_output<S>(span: &mut S, response: &IndexMap<String, Json>, choice: &Json)
where
    S: Span,
{
    let Some(message) = field(choice, "message").and_then(object) else {
        return;
    };
    if let Some(tool_calls) = message
        .get("tool_calls")
        .and_then(array)
        .filter(|calls| !calls.is_empty())
    {
        let transformed = tool_calls
            .iter()
            .map(|call| transformed_tool_call(response, call))
            .collect::<Vec<_>>();
        if let Ok(record) = safe_dumps(&transformed, DEFAULT_MAX_RECURSE_DEPTH, None) {
            span.safe_set_attribute(
                LangfuseSpanAttributes::ObservationOutput.value(),
                &OtelModelValue::String(record),
            );
        }
        return;
    }
    let mut output_data = IndexMap::new();
    if let Some(role) = message.get("role").filter(|value| truthiness(value)) {
        output_data.insert("role".to_owned(), role.clone());
    }
    if let Some(content) = message
        .get("content")
        .filter(|value| !matches!(value, Json::Null))
    {
        output_data.insert("content".to_owned(), content.clone());
    }
    if output_data.is_empty() {
        return;
    }
    if let Ok(record) = safe_dumps(&output_data, DEFAULT_MAX_RECURSE_DEPTH, None) {
        span.safe_set_attribute(
            LangfuseSpanAttributes::ObservationOutput.value(),
            &OtelModelValue::String(record),
        );
    }
}

fn transformed_tool_call(response: &IndexMap<String, Json>, tool_call: &Json) -> Json {
    let function = field(tool_call, "function").and_then(object);
    let arguments = match function.and_then(|fields| fields.get("arguments")) {
        Some(Json::Text(text)) => parse_arguments(text),
        Some(other) => other.clone(),
        None => Json::Object(IndexMap::new()),
    };
    let mut call = IndexMap::new();
    call.insert(
        "id".to_owned(),
        response
            .get("id")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    call.insert(
        "name".to_owned(),
        function
            .and_then(|fields| fields.get("name"))
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    call.insert(
        "call_id".to_owned(),
        field(tool_call, "id")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    call.insert("type".to_owned(), Json::Text("function_call".to_owned()));
    call.insert("arguments".to_owned(), arguments);
    Json::Object(call)
}

fn parse_arguments(text: &str) -> Json {
    match serde_json::from_str(text) {
        Ok(parsed) => parsed,
        Err(_source) => Json::Object(IndexMap::new()),
    }
}

fn set_output_items<S>(span: &mut S, output: &[Json])
where
    S: Span,
{
    let mut items = Vec::new();
    for item in output {
        let Some(item_type) = field(item, "type").and_then(super::wire::text) else {
            continue;
        };
        match item_type {
            "reasoning" => append_reasoning(item, &mut items),
            "message" => items.push(message_item(item)),
            "function_call" => items.push(function_call_item(item)),
            _ => {}
        }
    }
    if items.is_empty() {
        return;
    }
    if let Ok(record) = safe_dumps(&items, DEFAULT_MAX_RECURSE_DEPTH, None) {
        span.safe_set_attribute(
            LangfuseSpanAttributes::ObservationOutput.value(),
            &OtelModelValue::String(record),
        );
    }
}

fn append_reasoning(item: &Json, items: &mut Vec<Json>) {
    let Some(summaries) = field(item, "summary").and_then(array) else {
        return;
    };
    for summary in summaries {
        let Some(text) = field(summary, "text") else {
            continue;
        };
        let mut entry = IndexMap::new();
        entry.insert(
            "role".to_owned(),
            Json::Text("reasoning_summary".to_owned()),
        );
        entry.insert("content".to_owned(), text.clone());
        items.push(Json::Object(entry));
    }
}

fn message_item(item: &Json) -> Json {
    let role = field(item, "role")
        .cloned()
        .unwrap_or_else(|| Json::Text("assistant".to_owned()));
    let content = field(item, "content")
        .and_then(array)
        .and_then(|parts| parts.first())
        .and_then(|part| field(part, "text"))
        .cloned()
        .unwrap_or_else(|| Json::Text(String::new()));
    let mut entry = IndexMap::new();
    entry.insert("role".to_owned(), role);
    entry.insert("content".to_owned(), content);
    Json::Object(entry)
}

fn function_call_item(item: &Json) -> Json {
    let arguments = match field(item, "arguments") {
        Some(Json::Text(text)) => safe_arguments(text),
        Some(other) => other.clone(),
        None => Json::Object(IndexMap::new()),
    };
    let mut call = IndexMap::new();
    call.insert(
        "id".to_owned(),
        field(item, "id")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    call.insert(
        "name".to_owned(),
        field(item, "name")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    call.insert(
        "call_id".to_owned(),
        field(item, "call_id")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    call.insert("type".to_owned(), Json::Text("function_call".to_owned()));
    call.insert("arguments".to_owned(), arguments);
    Json::Object(call)
}

fn safe_arguments(text: &str) -> Json {
    let Ok(parsed) = safe_json_loads(text.as_bytes()) else {
        return Json::Object(IndexMap::new());
    };
    serde_json::from_value(parsed).unwrap_or_else(|_source| Json::Object(IndexMap::new()))
}
