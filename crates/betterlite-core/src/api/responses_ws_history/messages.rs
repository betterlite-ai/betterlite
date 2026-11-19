use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::ResponsesWsHistoryError;

const ASSISTANT: &str = "assistant";
const FUNCTION_CALL: &str = "function_call";
const INPUT_TEXT: &str = "input_text";
const MESSAGE: &str = "message";
const OUTPUT_TEXT: &str = "output_text";
const TEXT: &str = "text";

fn text_message(role: &Json, part_type: &str, text: &str) -> IndexMap<String, Json> {
    let mut part = IndexMap::new();
    part.insert("type".to_owned(), Json::Text(part_type.to_owned()));
    part.insert("text".to_owned(), Json::Text(text.to_owned()));
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text(MESSAGE.to_owned()));
    fields.insert("role".to_owned(), role.clone());
    fields.insert("content".to_owned(), Json::List(vec![Json::Object(part)]));
    fields
}

#[must_use]
pub fn input_to_messages(input: &Json) -> Vec<IndexMap<String, Json>> {
    match input {
        Json::Text(text) => vec![text_message(
            &Json::Text("user".to_owned()),
            INPUT_TEXT,
            text,
        )],
        Json::List(items) => items
            .iter()
            .filter_map(|item| match item {
                Json::Object(fields) => Some(fields.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            })
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Vec::new(),
    }
}

fn join_output_text(parts: &[Json]) -> Result<String, ResponsesWsHistoryError> {
    let mut text = String::new();
    for part in parts {
        let Json::Object(fields) = part else {
            continue;
        };
        let Some(Json::Text(part_type)) = fields.get("type") else {
            continue;
        };
        if part_type != OUTPUT_TEXT && part_type != TEXT {
            continue;
        }
        match fields.get("text") {
            None => {}
            Some(Json::Text(piece)) => text.push_str(piece),
            Some(_) => return Err(ResponsesWsHistoryError::shape("text", "a string")),
        }
    }
    Ok(text)
}

fn message_item(
    fields: &IndexMap<String, Json>,
) -> Result<Option<IndexMap<String, Json>>, ResponsesWsHistoryError> {
    let role = fields
        .get("role")
        .cloned()
        .unwrap_or_else(|| Json::Text(ASSISTANT.to_owned()));
    let parts = match fields.get("content") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::List(parts)) => parts,
        Some(_) => return Err(ResponsesWsHistoryError::shape("content", "a list")),
    };
    let text = join_output_text(parts)?;
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text_message(&role, OUTPUT_TEXT, &text)))
}

pub fn extract_output_messages(
    completed: &IndexMap<String, Json>,
) -> Result<Vec<IndexMap<String, Json>>, ResponsesWsHistoryError> {
    let Some(Json::Object(response)) = completed.get("response") else {
        return Ok(Vec::new());
    };
    let output = match response.get("output") {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::List(items)) => items,
        Some(_) => return Err(ResponsesWsHistoryError::shape("output", "a list")),
    };
    let mut messages = Vec::new();
    for item in output {
        let Json::Object(fields) = item else {
            continue;
        };
        match fields.get("type") {
            Some(Json::Text(item_type)) if item_type == MESSAGE => {
                if let Some(message) = message_item(fields)? {
                    messages.push(message);
                }
            }
            Some(Json::Text(item_type)) if item_type == FUNCTION_CALL => {
                messages.push(fields.clone());
            }
            _ => {}
        }
    }
    Ok(messages)
}
