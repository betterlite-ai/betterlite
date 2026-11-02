use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::has_value;

// litellm.integrations.galileo.GalileoObserve._galileo_input_messages.
#[must_use]
pub fn galileo_input_messages(messages: Option<&Json>, input_text: &str) -> Vec<Json> {
    let unwrapped = match messages {
        Some(Json::Object(fields)) => fields.get("messages"),
        other => other,
    };
    let Some(Json::List(items)) = unwrapped else {
        return vec![fallback_message(input_text)];
    };

    let built: Vec<Json> = items
        .iter()
        .filter_map(|message| {
            let Json::Object(fields) = message else {
                return None;
            };
            // str(role): a role is declared as a string on every message schema this task's cases send, so this reads it back without porting Python's repr formatting for other types.
            let role = match fields.get("role") {
                Some(role) if has_value(role) => match role {
                    Json::Text(text) => text.clone(),
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_) => String::new(),
                },
                _ => return None,
            };
            let mut entry = IndexMap::new();
            entry.insert("role".to_owned(), Json::Text(role));
            entry.insert(
                "content".to_owned(),
                Json::Text(convert_content_list_to_str(fields)),
            );
            Some(Json::Object(entry))
        })
        .collect();

    if built.is_empty() {
        vec![fallback_message(input_text)]
    } else {
        built
    }
}

fn fallback_message(input_text: &str) -> Json {
    let mut entry = IndexMap::new();
    entry.insert("role".to_owned(), Json::Text("user".to_owned()));
    entry.insert("content".to_owned(), Json::Text(input_text.to_owned()));
    Json::Object(entry)
}

// litellm.litellm_core_utils.prompt_templates.common_utils.convert_content_list_to_str.
#[must_use]
pub fn convert_content_list_to_str(message: &IndexMap<String, Json>) -> String {
    let mut text = match message.get("content") {
        Some(Json::List(blocks)) => blocks
            .iter()
            .map(block_text_or_default)
            .filter(|block_text| !block_text.is_empty())
            .collect(),
        Some(Json::Text(content)) => content.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => {
            String::new()
        }
    };
    text.push_str(&extract_search_results_text(message.get("search_results")));
    text
}

// The "text" field of a content block, read back as given with no missing-field fallback text.
fn block_text_or_default(block: &Json) -> String {
    match block {
        Json::Object(fields) => match fields.get("text") {
            Some(Json::Text(text)) => text.clone(),
            _ => String::new(),
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            String::new()
        }
    }
}

fn push_search_result_text(text: &mut String, fields: &IndexMap<String, Json>) {
    for key in ["source", "title"] {
        if let Some(Json::Text(value)) = fields.get(key) {
            text.push_str(value);
        }
    }
    if let Some(Json::List(blocks)) = fields.get("content") {
        for block in blocks {
            text.push_str(&block_text_or_default(block));
        }
    }
    if let Some(citations) = fields
        .get("citations")
        .filter(|value| !matches!(value, Json::Null))
    {
        text.push_str(&serde_json::to_string(citations).unwrap_or_default());
    }
}

// litellm.litellm_core_utils.prompt_templates.common_utils.extract_search_results_text.
#[must_use]
pub fn extract_search_results_text(search_results: Option<&Json>) -> String {
    let Some(Json::List(results)) = search_results else {
        return String::new();
    };
    let mut text = String::new();
    for result in results {
        let Json::Object(fields) = result else {
            continue;
        };
        push_search_result_text(&mut text, fields);
    }
    text
}

// litellm.integrations.galileo.GalileoObserve._input_text_from_messages.
#[must_use]
pub fn input_text_from_messages(messages: &Json) -> String {
    match messages {
        Json::Text(text) => text.clone(),
        Json::List(items) => find_role_content(items.iter().rev(), true)
            .or_else(|| find_role_content(items.iter(), false))
            .unwrap_or_default(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => String::new(),
    }
}

fn find_role_content<'items, I>(items: I, only_user_or_human: bool) -> Option<String>
where
    I: Iterator<Item = &'items Json>,
{
    for message in items {
        let Json::Object(fields) = message else {
            continue;
        };
        if only_user_or_human {
            let role = match fields.get("role") {
                Some(Json::Text(role)) => role.to_lowercase(),
                _ => String::new(),
            };
            if role != "user" && role != "human" {
                continue;
            }
        }
        let content = message_content_text(fields);
        if !content.is_empty() {
            return Some(content);
        }
    }
    None
}

fn message_content_text(fields: &IndexMap<String, Json>) -> String {
    let Some(content) = fields.get("content") else {
        return String::new();
    };
    let Json::List(blocks) = content else {
        return match content {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                String::new()
            }
        };
    };
    let parts: Vec<String> = blocks.iter().map(block_text_or_default).collect();
    parts.join(" ")
}
