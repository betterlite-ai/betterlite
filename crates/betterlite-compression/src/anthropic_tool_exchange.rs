use serde_json::Value;
use std::collections::BTreeSet;

pub fn extract_tool_use_ids(content: &Value) -> Vec<String> {
    let ids = extract_ids(content, "tool_use", "id")
        .into_iter()
        .map(str::to_owned)
        .collect();
    ids
}

fn extract_ids<'content>(
    content: &'content Value,
    block_type: &str,
    id_field: &str,
) -> Vec<&'content str> {
    let ids = content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some(block_type))
                .filter_map(|part| part.get(id_field).and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default();
    ids
}

pub fn extract_tool_result_ids(content: &Value) -> BTreeSet<String> {
    let ids = content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("tool_result"))
                .filter_map(|part| part.get("tool_use_id").and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    ids
}

pub fn extract_exchange_spans(messages: &[Value]) -> (Vec<Vec<usize>>, Option<&'static str>) {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let current = &messages[index];
        if current.get("role") != Some(&Value::String("assistant".to_owned())) {
            index = index.saturating_add(1);
            continue;
        }
        let tool_ids = extract_tool_use_ids(current.get("content").unwrap_or(&Value::Null));
        if tool_ids.is_empty() {
            index = index.saturating_add(1);
            continue;
        }
        let Some(next) = messages.get(index + 1) else {
            return (Vec::new(), Some("invalid_anthropic_tool_sequence"));
        };
        if next.get("role") != Some(&Value::String("user".to_owned())) {
            return (Vec::new(), Some("invalid_anthropic_tool_sequence"));
        }
        let result_ids = extract_tool_result_ids(next.get("content").unwrap_or(&Value::Null));
        if result_ids.is_empty() || tool_ids.iter().any(|id| !result_ids.contains(id)) {
            return (Vec::new(), Some("invalid_anthropic_tool_sequence"));
        }
        spans.push(vec![index, index + 1]);
        index = index.saturating_add(2);
    }
    (spans, None)
}

mod tests;
