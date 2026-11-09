use crate::arguments::json::{Json, Object};
use crate::message_content::extract_search_results_text;

pub(super) const CONTENT: &str = "content";
pub(super) const ROLE: &str = "role";
pub(super) const NAME: &str = "name";
pub(super) const TYPE: &str = "type";
pub(super) const TEXT: &str = "text";
pub(super) const TEXT_TYPE: &str = "text";
pub(super) const SEARCH_RESULTS: &str = "search_results";
pub(super) const DEFAULT_CONTINUE_TEXT: &str = "Please continue.";

// str.strip()'s whitespace set also covers U+001C-U+001F; Rust's is_whitespace() does not.
pub(super) fn is_blank(text: &str) -> bool {
    text.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    })
    .is_empty()
}

pub(super) fn has_non_content_values(message: &Object) -> bool {
    message.iter().any(|(key, value)| {
        !matches!(key.as_str(), CONTENT | ROLE | NAME) && !matches!(value, Json::Null)
    })
}

pub(super) fn content_list_to_str(message: &Object) -> String {
    let mut text = match message.get(CONTENT) {
        Some(Json::Text(content)) => content.clone(),
        Some(Json::List(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.as_object()
                    .and_then(|fields| fields.get(TEXT))
                    .and_then(Json::as_str)
            })
            .collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => {
            String::new()
        }
    };
    text.push_str(&extract_search_results_text(message.get(SEARCH_RESULTS)));
    text
}
