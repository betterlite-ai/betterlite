use super::{choice_message, optional_text};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::helpers::dicts;
use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

const TOOL_CALL_TYPES: [&str; 2] = ["function_call", "custom_tool_call"];

pub(super) fn responses_choices(response: &MetadataMap) -> Vec<MetadataMap> {
    let Some(output) = response.get("output") else {
        return Vec::new();
    };
    let items = dicts(output);
    let messages: Vec<&MetadataMap> = items
        .iter()
        .filter(|item| item.get("type").and_then(OtelModelValue::as_str) == Some("message"))
        .collect();
    let tool_calls: Vec<MetadataMap> = items
        .iter()
        .filter(|item| {
            item.get("type")
                .and_then(OtelModelValue::as_str)
                .is_some_and(|kind| TOOL_CALL_TYPES.contains(&kind))
        })
        .map(tool_call)
        .collect();
    if messages.is_empty() && tool_calls.is_empty() {
        return Vec::new();
    }
    let parts = message_parts(&messages);
    let content = parts_text(&parts, "output_text", "text");
    let refusal = parts_text(&parts, "refusal", "refusal");
    let has_tool_calls = !tool_calls.is_empty();
    let message = choice_message(
        first_role(&messages),
        content.as_deref(),
        refusal.as_deref(),
        has_tool_calls.then_some(tool_calls),
    );
    let finish_reason = responses_finish_reason(response, has_tool_calls);
    vec![BTreeMap::from([
        ("finish_reason".to_owned(), optional_text(finish_reason)),
        ("message".to_owned(), OtelModelValue::Object(message)),
    ])]
}

fn message_parts(messages: &[&MetadataMap]) -> Vec<MetadataMap> {
    let mut parts = Vec::new();
    for message in messages {
        if let Some(content) = message.get("content") {
            parts.extend(dicts(content));
        }
    }
    parts
}

fn first_role<'item>(messages: &[&'item MetadataMap]) -> &'item str {
    messages
        .iter()
        .find_map(|message| {
            message
                .get("role")
                .and_then(OtelModelValue::as_str)
                .filter(|role| !role.is_empty())
        })
        .unwrap_or("assistant")
}

fn parts_text(parts: &[MetadataMap], part_type: &str, field: &str) -> Option<String> {
    let mut joined = String::new();
    let mut found = false;
    for part in parts {
        if part.get("type").and_then(OtelModelValue::as_str) != Some(part_type) {
            continue;
        }
        let Some(text) = part.get(field).and_then(OtelModelValue::as_str) else {
            continue;
        };
        joined.push_str(text);
        found = true;
    }
    found.then_some(joined)
}

fn tool_call(item: &MetadataMap) -> MetadataMap {
    let custom = item.get("type").and_then(OtelModelValue::as_str) == Some("custom_tool_call");
    let name = item
        .get("name")
        .and_then(OtelModelValue::as_str)
        .unwrap_or_default();
    let id = ["call_id", "id"]
        .into_iter()
        .find_map(|key| {
            item.get(key)
                .and_then(OtelModelValue::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    let arguments = item
        .get(if custom { "input" } else { "arguments" })
        .and_then(OtelModelValue::as_str)
        .unwrap_or_default();
    let function = BTreeMap::from([
        (
            "arguments".to_owned(),
            OtelModelValue::String(arguments.to_owned()),
        ),
        ("name".to_owned(), OtelModelValue::String(name.to_owned())),
    ]);
    BTreeMap::from([
        ("function".to_owned(), OtelModelValue::Object(function)),
        ("id".to_owned(), OtelModelValue::String(id.to_owned())),
        (
            "type".to_owned(),
            OtelModelValue::String("function".to_owned()),
        ),
    ])
}

fn responses_finish_reason(response: &MetadataMap, has_tool_calls: bool) -> Option<&str> {
    match response.get("status").and_then(OtelModelValue::as_str) {
        Some("completed") => Some(if has_tool_calls { "tool_calls" } else { "stop" }),
        Some("incomplete") => {
            let reason = response
                .get("incomplete_details")
                .and_then(OtelModelValue::as_object)
                .and_then(|details| details.get("reason"))
                .and_then(OtelModelValue::as_str);
            Some(if reason == Some("content_filter") {
                "content_filter"
            } else {
                "length"
            })
        }
        Some(_) | None => None,
    }
}
