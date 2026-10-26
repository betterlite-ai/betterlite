use serde_json::Value;
use std::collections::BTreeSet;

pub fn extract_exchange_spans(messages: &[Value]) -> (Vec<Vec<usize>>, Option<&'static str>) {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let current = &messages[index];
        let tool_ids: Vec<&str> = current
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("tool_use"))
            .filter_map(|part| part.get("id").and_then(Value::as_str))
            .filter(|id| !id.is_empty())
            .collect();
        if current.get("role").and_then(Value::as_str) != Some("assistant") || tool_ids.is_empty() {
            index = index.saturating_add(1);
            continue;
        }
        let Some(next) = messages
            .get(index + 1)
            .filter(|next| next.get("role").and_then(Value::as_str) == Some("user"))
        else {
            return (Vec::new(), Some("invalid_anthropic_tool_sequence"));
        };
        let result_ids: BTreeSet<&str> = next
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|part| part.get("tool_use_id").and_then(Value::as_str))
            .filter(|id| !id.is_empty())
            .collect();
        if result_ids.is_empty() || tool_ids.iter().any(|id| !result_ids.contains(id)) {
            return (Vec::new(), Some("invalid_anthropic_tool_sequence"));
        }
        spans.push(vec![index, index + 1]);
        index = index.saturating_add(2);
    }
    (spans, None)
}

mod tests;
