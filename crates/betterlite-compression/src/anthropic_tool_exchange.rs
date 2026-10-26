use serde_json::Value;
use std::collections::BTreeSet;

pub fn extract_tool_use_ids(content: &Value) -> Vec<String> {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type") == Some(&Value::String("tool_use".to_owned())))
                .filter_map(|part| part.get("id").and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn extract_tool_result_ids(content: &Value) -> BTreeSet<String> {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type") == Some(&Value::String("tool_result".to_owned())))
                .filter_map(|part| part.get("tool_use_id").and_then(Value::as_str))
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn extract_anthropic_tool_exchange_spans(
    messages: &[Value],
) -> (Vec<Vec<usize>>, Option<&'static str>) {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let current = &messages[index];
        if current.get("role") != Some(&Value::String("assistant".to_owned())) {
            index += 1;
            continue;
        }
        let tool_ids = extract_tool_use_ids(current.get("content").unwrap_or(&Value::Null));
        if tool_ids.is_empty() {
            index += 1;
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
        index += 2;
    }
    (spans, None)
}

#[cfg(test)]
mod tests;
