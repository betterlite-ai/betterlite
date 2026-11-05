use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::arguments::json::{Json, Object};

use super::HistoryError;
use super::groups::declared_tool_call_ids;
use super::text::sanitize_empty_text_content;

pub const MAX_HISTORY_MESSAGES: usize = 1024;
pub const MAX_TOOL_CALLS_PER_MESSAGE: usize = 1024;

struct MatchingResults {
    messages: Vec<Json>,
    indices: Vec<usize>,
    ids: HashSet<String>,
}

struct MissingRepair {
    messages: Vec<Json>,
    consumed_indices: Vec<usize>,
    consumed_count: usize,
}

#[must_use]
pub fn add_missing_tool_results(
    current_message: &Json,
    messages: &[Json],
    current_index: usize,
) -> (Vec<Json>, usize) {
    let expected_ids = declared_tool_call_ids(current_message);
    let Some(repair) = missing_repair(current_message, &expected_ids, messages, current_index)
    else {
        return (vec![current_message.clone()], 0);
    };
    (repair.messages, repair.consumed_count)
}

#[must_use]
pub fn is_orphaned_tool_result(message: &Json, previous: &[Json]) -> bool {
    let latest_assistant_ids = previous
        .iter()
        .rev()
        .find(|item| message_field_text(item, "role") == Some("assistant"))
        .map(|assistant| declared_tool_call_ids(assistant).into_iter().collect());
    tool_result_is_orphaned(message, latest_assistant_ids.as_ref())
}

fn tool_result_is_orphaned(message: &Json, latest_assistant_ids: Option<&HashSet<String>>) -> bool {
    if !matches!(
        message_field_text(message, "role"),
        Some("tool" | "function")
    ) {
        return false;
    }
    let Some(tool_id) = message_field_text(message, "tool_call_id").filter(|id| !id.is_empty())
    else {
        return false;
    };
    latest_assistant_ids.is_none_or(|declared_ids| !declared_ids.contains(tool_id))
}

pub fn sanitize_messages(
    messages: &[Json],
    modify_params: bool,
) -> Result<Vec<Json>, HistoryError> {
    if !modify_params {
        return Ok(messages.to_vec());
    }
    enforce_input_caps(messages)?;
    let mut sanitized = Vec::new();
    let mut consumed = BTreeSet::new();
    let mut latest_assistant_ids: Option<HashSet<String>> = None;
    for (index, message) in messages.iter().enumerate() {
        if consumed.contains(&index) {
            continue;
        }
        if message.as_object().is_none() {
            return Err(HistoryError::MessageNotObject { index });
        }
        let normalized = sanitize_empty_text_content(message);
        if message_field_text(message, "role") == Some("assistant") {
            let expected_ids = declared_tool_call_ids(&normalized);
            latest_assistant_ids = Some(expected_ids.iter().cloned().collect());
            if let Some(repair) = missing_repair(&normalized, &expected_ids, messages, index) {
                consumed.extend(repair.consumed_indices);
                sanitized.extend(repair.messages);
                continue;
            }
            sanitized.push(normalized);
            continue;
        }
        if tool_result_is_orphaned(&normalized, latest_assistant_ids.as_ref()) {
            continue;
        }
        sanitized.push(normalized);
    }
    Ok(deduplicate_results(sanitized))
}

fn enforce_input_caps(messages: &[Json]) -> Result<(), HistoryError> {
    if messages.len() > MAX_HISTORY_MESSAGES {
        return Err(HistoryError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES,
        });
    }
    for message in messages {
        if let Some(Json::List(calls)) = message
            .as_object()
            .and_then(|fields| fields.get("tool_calls"))
            && calls.len() > MAX_TOOL_CALLS_PER_MESSAGE
        {
            return Err(HistoryError::TooManyToolCalls {
                limit: MAX_TOOL_CALLS_PER_MESSAGE,
            });
        }
    }
    Ok(())
}

fn missing_repair(
    current_message: &Json,
    expected_ids: &[String],
    messages: &[Json],
    current_index: usize,
) -> Option<MissingRepair> {
    if expected_ids.is_empty() {
        return None;
    }
    let expected_id_set: HashSet<&str> = expected_ids.iter().map(String::as_str).collect();
    let actual = collect_matching_results(messages, current_index, &expected_id_set);
    let missing_ids = expected_ids
        .iter()
        .filter(|expected_id| !actual.ids.contains(expected_id.as_str()))
        .collect::<Vec<_>>();
    if missing_ids.is_empty() {
        return None;
    }
    let mut repaired = vec![current_message.clone()];
    repaired.extend(actual.messages.iter().cloned());
    append_missing_results(&mut repaired, current_message, missing_ids);
    Some(MissingRepair {
        messages: repaired,
        consumed_indices: actual.indices,
        consumed_count: actual.messages.len(),
    })
}

fn collect_matching_results(
    messages: &[Json],
    current_index: usize,
    expected_ids: &HashSet<&str>,
) -> MatchingResults {
    let mut matching_messages = Vec::new();
    let mut matching_indices = Vec::new();
    let mut matching_ids = HashSet::new();
    for (candidate_index, candidate) in messages
        .iter()
        .enumerate()
        .skip(current_index.saturating_add(1))
    {
        if message_field_text(candidate, "role") == Some("assistant") {
            break;
        }
        if !matches!(
            message_field_text(candidate, "role"),
            Some("tool" | "function")
        ) {
            continue;
        }
        let Some(tool_id) = message_field_text(candidate, "tool_call_id") else {
            continue;
        };
        if !expected_ids.contains(tool_id) {
            continue;
        }
        matching_messages.push(candidate.clone());
        matching_indices.push(candidate_index);
        matching_ids.insert(tool_id.to_owned());
    }
    MatchingResults {
        messages: matching_messages,
        indices: matching_indices,
        ids: matching_ids,
    }
}

fn append_missing_results(
    repaired: &mut Vec<Json>,
    current_message: &Json,
    missing_ids: Vec<&String>,
) {
    for missing_id in missing_ids {
        repaired.push(missing_result(current_message, missing_id));
    }
}

fn missing_result(current_message: &Json, missing_id: &str) -> Json {
    let tool_name = get_tool_name(current_message, missing_id);
    let result_content = format!(
        "[System: Tool execution skipped/interrupted by user. No result provided for tool '{tool_name}'.]"
    );
    let mut fields = Object::new();
    fields.insert("role".to_owned(), Json::Text("tool".to_owned()));
    fields.insert("tool_call_id".to_owned(), Json::Text(missing_id.to_owned()));
    fields.insert("content".to_owned(), Json::Text(result_content));
    Json::Object(fields)
}

fn get_tool_name<'message>(current_message: &'message Json, missing_id: &str) -> &'message str {
    let Some(Json::List(calls)) = current_message
        .as_object()
        .and_then(|fields| fields.get("tool_calls"))
    else {
        return "unknown_tool";
    };
    let Some(tool_call) = calls.iter().find(|call| {
        call.as_object()
            .and_then(|fields| fields.get("id"))
            .and_then(Json::as_str)
            == Some(missing_id)
    }) else {
        return "unknown_tool";
    };
    tool_call
        .as_object()
        .and_then(|fields| fields.get("function"))
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("name"))
        .and_then(Json::as_str)
        .unwrap_or("unknown_tool")
}

fn message_field_text<'message>(message: &'message Json, field: &str) -> Option<&'message str> {
    message
        .as_object()
        .and_then(|fields| fields.get(field))
        .and_then(Json::as_str)
}

fn deduplicate_results(messages: Vec<Json>) -> Vec<Json> {
    let mut remove_indices = BTreeSet::new();
    let mut seen_ids = BTreeMap::new();
    for (index, message) in messages.iter().enumerate() {
        if matches!(
            message_field_text(message, "role"),
            Some("tool" | "function")
        ) {
            if let Some(tool_id) =
                message_field_text(message, "tool_call_id").filter(|tool_id| !tool_id.is_empty())
                && let Some(previous_index) = seen_ids.insert(tool_id.to_owned(), index)
            {
                remove_indices.insert(previous_index);
            }
        } else {
            seen_ids.clear();
        }
    }
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| (!remove_indices.contains(&index)).then_some(message))
        .collect()
}
