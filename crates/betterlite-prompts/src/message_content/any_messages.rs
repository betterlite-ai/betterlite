use crate::arguments::json::{Json, Object};

use super::extract_search_results_text;

// LiteLLM's signature takes `object`, so this works on raw JSON rather than the typed `Message`.
#[must_use]
pub fn to_str_messages(messages: &Json) -> Json {
    match messages {
        Json::List(items) => try_convert_all(items).map_or_else(
            || {
                // Here LiteLLM falls back to `json.dumps` per element, not `str()`, so a string comes back quoted.
                Json::List(
                    items
                        .iter()
                        .map(|item| {
                            input_entry(Json::Text(serde_json::to_string(item).unwrap_or_default()))
                        })
                        .collect(),
                )
            },
            Json::List,
        ),
        Json::Text(text) => Json::List(vec![input_entry(Json::Text(text.clone()))]),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            Json::List(vec![input_entry(Json::Text(
                serde_json::to_string(other).unwrap_or_default(),
            ))])
        }
    }
}

fn try_convert_all(items: &[Json]) -> Option<Vec<Json>> {
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(fields) = item else {
            return None;
        };
        let mut message = fields.clone();
        let text = content_list_to_str(&message)?;
        if !text.is_empty() {
            message.insert("content".to_owned(), Json::Text(text));
        }
        converted.push(Json::Object(message));
    }
    Some(converted)
}

fn content_list_to_str(message: &Object) -> Option<String> {
    let mut text = match message.get("content") {
        Some(Json::List(parts)) => {
            let mut parts_text = String::new();
            for part in parts {
                let Json::Object(fields) = part else {
                    return None;
                };
                if let Some(value) = fields.get("text").and_then(Json::as_str) {
                    parts_text.push_str(value);
                }
            }
            parts_text
        }
        Some(Json::Text(value)) => value.clone(),
        _ => String::new(),
    };
    text.push_str(&extract_search_results_text(message.get("search_results")));
    Some(text)
}

fn input_entry(value: Json) -> Json {
    let entry = Object::from_iter([("input".to_owned(), value)]);
    Json::Object(entry)
}

mod tests;
