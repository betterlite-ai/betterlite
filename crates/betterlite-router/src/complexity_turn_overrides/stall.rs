use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolCallEvent {
    name: String,
    arguments: String,
    is_error: Option<bool>,
}

// _json_arguments and _tool_call_signature: both tool-call surfaces canonicalize to one shape
fn canonical_arguments(value: &Json) -> String {
    if let Json::Text(text) = value
        && let Ok(parsed) = serde_json::from_str::<Json>(text)
    {
        return canonical(&parsed);
    }
    canonical(value)
}

// a length-prefixed encoding keeps nested values unambiguous without serializing through serde
fn canonical(value: &Json) -> String {
    match value {
        Json::Null => "n".to_owned(),
        Json::Bool(true) => "t".to_owned(),
        Json::Bool(false) => "f".to_owned(),
        Json::Number(number) => format!("d{number}"),
        Json::Text(text) => format!("s{}:{text}", text.chars().count()),
        Json::List(items) => {
            let mut encoded = format!("l{}", items.len());
            for item in items {
                encoded.push('(');
                encoded.push_str(&canonical(item));
                encoded.push(')');
            }
            encoded
        }
        Json::Object(fields) => {
            let mut sorted: Vec<(&String, &Json)> = fields.iter().collect();
            sorted.sort_by(|left, right| left.0.cmp(right.0));
            let mut encoded = format!("o{}", sorted.len());
            for (key, item) in sorted {
                encoded.push_str("(s");
                encoded.push_str(&key.chars().count().to_string());
                encoded.push(':');
                encoded.push_str(key);
                encoded.push(':');
                encoded.push_str(&canonical(item));
                encoded.push(')');
            }
            encoded
        }
    }
}

// the last result for a call id wins, as the source dict construction does
fn error_by_call_id(messages: &[Json]) -> BTreeMap<String, bool> {
    let mut errors = BTreeMap::new();
    for message in messages {
        let Json::Object(fields) = message else {
            continue;
        };
        if !matches!(fields.get("role"), Some(Json::Text(role)) if role == "user") {
            continue;
        }
        let Some(Json::List(content)) = fields.get("content") else {
            continue;
        };
        for part in content {
            let Json::Object(part) = part else {
                continue;
            };
            if !matches!(part.get("type"), Some(Json::Text(kind)) if kind == "tool_result") {
                continue;
            }
            let Some(Json::Text(call_id)) = part.get("tool_use_id") else {
                continue;
            };
            let is_error = matches!(part.get("is_error"), Some(Json::Bool(true)));
            errors.insert(call_id.clone(), is_error);
        }
    }
    errors
}

// _iter_tool_call_events_newest_first: the assistant's own calls, newest first
fn events_newest_first(messages: &[Json]) -> Vec<ToolCallEvent> {
    let errors = error_by_call_id(messages);
    let mut events = Vec::new();
    for message in messages.iter().rev() {
        let Json::Object(fields) = message else {
            continue;
        };
        if !matches!(fields.get("role"), Some(Json::Text(role)) if role == "assistant") {
            continue;
        }
        if let Some(Json::List(content)) = fields.get("content") {
            for part in content.iter().rev() {
                tool_use_event(part, &errors, &mut events);
            }
        }
        if let Some(Json::List(tool_calls)) = fields.get("tool_calls") {
            for call in tool_calls.iter().rev() {
                tool_call_event(call, &mut events);
            }
        }
    }
    events
}

// the Anthropic surface: a tool_use block carries the input object and its result carries the error flag
fn tool_use_event(part: &Json, errors: &BTreeMap<String, bool>, events: &mut Vec<ToolCallEvent>) {
    let Json::Object(fields) = part else {
        return;
    };
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "tool_use") {
        return;
    }
    let Some(Json::Text(name)) = fields.get("name") else {
        return;
    };
    let is_error = match fields.get("id") {
        Some(Json::Text(call_id)) => errors.get(call_id).copied(),
        _ => None,
    };
    events.push(ToolCallEvent {
        name: name.clone(),
        arguments: fields
            .get("input")
            .map_or_else(|| canonical(&Json::Null), canonical_arguments),
        is_error,
    });
}

// the chat surface: a tool_calls entry carries JSON-string arguments and no error status
fn tool_call_event(call: &Json, events: &mut Vec<ToolCallEvent>) {
    let Json::Object(call_fields) = call else {
        return;
    };
    let Some(Json::Object(function)) = call_fields.get("function") else {
        return;
    };
    let Some(Json::Text(name)) = function.get("name") else {
        return;
    };
    events.push(ToolCallEvent {
        name: name.clone(),
        arguments: function
            .get("arguments")
            .map_or_else(|| canonical(&Json::Null), canonical_arguments),
        is_error: None,
    });
}

// detect_stalled_task: the newest call repeats, or errors, repeat_threshold times across the window
#[must_use]
pub fn detect_stalled_task(messages: Option<&[Json]>, window: u32, repeat_threshold: u32) -> bool {
    let Some(messages) = messages else {
        return false;
    };
    if repeat_threshold == 0 {
        return false;
    }
    let events = events_newest_first(messages);
    let limit = usize::try_from(window).unwrap_or(usize::MAX);
    let threshold = usize::try_from(repeat_threshold).unwrap_or(usize::MAX);
    let recent: Vec<&ToolCallEvent> = events.iter().take(limit).collect();
    if recent.len() < threshold {
        return false;
    }
    let Some(newest) = recent.first() else {
        return false;
    };
    let repeats = recent
        .iter()
        .filter(|event| event.name == newest.name && event.arguments == newest.arguments)
        .count();
    if repeats >= threshold {
        return true;
    }
    newest.is_error == Some(true)
        && recent
            .iter()
            .filter(|event| event.is_error == Some(true))
            .count()
            >= threshold
}
