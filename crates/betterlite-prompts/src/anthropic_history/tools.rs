use serde_json::{Value, json};

use super::HistoryError;
use super::groups::declared_tool_call_ids;
use super::text::sanitize_empty_text_content;

pub fn add_missing_tool_results(
    current_message: &Value,
    messages: &[Value],
    current_index: usize,
) -> (Vec<Value>, usize) {
    let Some(calls) = current_message.get("tool_calls") else {
        return (vec![current_message.clone()], 0);
    };
    let Some(calls) = calls.as_array() else {
        return (vec![current_message.clone()], 0);
    };
    let ids = declared_tool_call_ids(current_message);
    if ids.is_empty() {
        return (vec![current_message.clone()], 0);
    }
    let mut found = Vec::new();
    let mut actual = Vec::new();
    for result in messages.iter().skip(current_index.saturating_add(1)) {
        match result.get("role").and_then(Value::as_str) {
            Some("assistant") => break,
            Some("tool" | "function") => {
                if let Some(id) = result.get("tool_call_id").and_then(Value::as_str)
                    && ids.iter().any(|expected| expected == id)
                {
                    found.push(id.to_owned());
                    actual.push(result.clone());
                }
            }
            _ => {}
        }
    }
    let missing = ids
        .iter()
        .filter(|id| !found.contains(id))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return (vec![current_message.clone()], 0);
    }
    let mut result = vec![current_message.clone()];
    result.extend(actual.iter().cloned());
    for id in missing {
        let name = calls
            .iter()
            .find(|call| call.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|call| call.pointer("/function/name"))
            .and_then(Value::as_str)
            .unwrap_or("unknown_tool");
        result.push(json!({
            "role":"tool",
            "tool_call_id":id,
            "content":format!("[System: Tool execution skipped/interrupted by user. No result provided for tool '{name}'.]")
        }));
    }
    (result, actual.len())
}

pub fn is_orphaned_tool_result(message: &Value, previous: &[Value]) -> bool {
    let role = message.get("role").and_then(Value::as_str);
    if !matches!(role, Some("tool" | "function")) {
        return false;
    }
    let Some(id) = message
        .get("tool_call_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return false;
    };
    let Some(assistant) = previous
        .iter()
        .rev()
        .find(|item| item.get("role").and_then(Value::as_str) == Some("assistant"))
    else {
        return true;
    };
    !declared_tool_call_ids(assistant)
        .iter()
        .any(|declared| declared == id)
}

pub fn sanitize_messages(
    messages: &[Value],
    modify_params: bool,
) -> Result<Vec<Value>, HistoryError> {
    if !modify_params {
        return Ok(messages.to_vec());
    }
    let mut sanitized = Vec::new();
    let mut consumed = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if consumed.contains(&index) {
            continue;
        }
        let Some(object) = message.as_object() else {
            return Err(HistoryError::MessageNotObject { index });
        };
        let normalized = sanitize_empty_text_content(message);
        if object.get("role").and_then(Value::as_str) == Some("assistant") {
            let ids = declared_tool_call_ids(message);
            if !ids.is_empty() {
                let mut actual = Vec::new();
                let mut actual_indices = Vec::new();
                let mut found = Vec::new();
                for (candidate_index, candidate) in messages.iter().enumerate().skip(index + 1) {
                    match candidate.get("role").and_then(Value::as_str) {
                        Some("assistant") => break,
                        Some("tool" | "function") => {
                            if let Some(id) = candidate.get("tool_call_id").and_then(Value::as_str)
                                && ids.iter().any(|expected| expected == id)
                            {
                                actual.push(candidate.clone());
                                found.push(id.to_owned());
                                actual_indices.push(candidate_index);
                            }
                        }
                        _ => {}
                    }
                }
                let missing = ids
                    .iter()
                    .filter(|id| !found.contains(id))
                    .collect::<Vec<_>>();
                if !missing.is_empty() {
                    consumed.extend(actual_indices);
                    sanitized.push(normalized);
                    sanitized.extend(actual);
                    sanitized.extend(missing.into_iter().map(|id| missing_result(message, id)));
                    continue;
                }
            }
        }
        if is_orphaned_tool_result(message, &sanitized) {
            continue;
        }
        sanitized.push(normalized);
    }
    Ok(deduplicate_results(sanitized))
}

fn missing_result(message: &Value, id: &str) -> Value {
    let name = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|call| call.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|call| call.pointer("/function/name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown_tool");
    json!({
        "role": "tool",
        "tool_call_id": id,
        "content": format!("[System: Tool execution skipped/interrupted by user. No result provided for tool '{name}'.]")
    })
}

fn deduplicate_results(messages: Vec<Value>) -> Vec<Value> {
    let mut remove = Vec::new();
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (index, message) in messages.iter().enumerate() {
        let role = message.get("role").and_then(Value::as_str);
        if matches!(role, Some("tool" | "function")) {
            if let Some(id) = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                && let Some(previous) = seen.insert(id.to_owned(), index)
            {
                remove.push(previous);
            }
        } else {
            seen.clear();
        }
    }
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| (!remove.contains(&index)).then_some(message))
        .collect()
}
