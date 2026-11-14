use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::{Message, get_str_from_messages};

use super::RequestData;
use super::services::{ResponsesInput, ResponsesInputRequest};

#[must_use]
pub fn selected_text_chars(
    messages: Option<&Json>,
    prompt: Option<&Json>,
    input: Option<&Json>,
) -> usize {
    if let Some(Json::List(items)) = messages
        && !items.is_empty()
    {
        return get_str_from_messages(&decoded_messages(items))
            .chars()
            .count();
    }
    text_or_list_chars(prompt)
        .or_else(|| text_or_list_chars(input))
        .unwrap_or(0)
}

fn text_or_list_chars(value: Option<&Json>) -> Option<usize> {
    match value {
        Some(Json::Text(text)) => Some(text.chars().count()),
        Some(Json::List(items)) => Some(item_chars(items)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => None,
    }
}

fn item_chars(items: &[Json]) -> usize {
    let mut total = 0_usize;
    for item in items {
        let chars = match item {
            Json::Text(text) => text.chars().count(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                serde_json::to_string(item).map_or(0, |text| text.chars().count())
            }
        };
        total = total.saturating_add(chars);
    }
    total
}

fn decoded_messages(items: &[Json]) -> Vec<Message> {
    let mut decoded = Vec::with_capacity(items.len());
    for item in items {
        let Ok(value) = serde_json::to_value(item) else {
            continue;
        };
        let Ok(message) = serde_json::from_value::<Message>(value) else {
            continue;
        };
        decoded.push(message);
    }
    decoded
}

#[must_use]
pub fn json_list(value: &Json) -> Option<&[Json]> {
    match value {
        Json::List(items) => Some(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn pretokenized_embedding_tokens(value: &Json) -> Option<i64> {
    let Json::List(items) = value else {
        return None;
    };
    if items.iter().all(is_token_id) {
        return i64::try_from(items.len()).ok();
    }
    let mut total: i64 = 0;
    for item in items {
        let Json::List(ids) = item else {
            return None;
        };
        if !ids.iter().all(is_token_id) {
            return None;
        }
        total = total.saturating_add(i64::try_from(ids.len()).unwrap_or(i64::MAX));
    }
    Some(total)
}

fn is_token_id(value: &Json) -> bool {
    match value {
        Json::Number(number) => number.as_i64().is_some() || number.as_u64().is_some(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}

#[must_use]
pub fn rerank_input_to_text(data: &RequestData) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(part) = data.get("query").and_then(rerank_part_text) {
        parts.push(part);
    }
    if let Some(Json::List(documents)) = data.get("documents") {
        for document in documents {
            if let Some(part) = rerank_part_text(document) {
                parts.push(part);
            }
        }
    }
    parts.join("\n")
}

fn rerank_part_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Object(_) => serde_json::to_string(value).ok(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn responses_chat_messages(responses: &dyn ResponsesInput, data: &RequestData) -> Vec<Json> {
    let empty_input = Json::Text(String::new());
    let input = match data.get("input") {
        Some(value) if !matches!(value, Json::Null) => value,
        Some(_) | None => &empty_input,
    };
    let instructions = match data.get("instructions") {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(_) | None => None,
    };
    responses.transform_responses_api_input(&ResponsesInputRequest {
        input,
        instructions,
    })
}
