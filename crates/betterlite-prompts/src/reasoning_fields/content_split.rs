use crate::message_content::{Content, Message};
use fancy_regex::Regex;

const REASONING_TAG_PATTERN: &str =
    "(?s)^<(?:think|thinking|budget:thinking)>(.*?)</(?:think|thinking|budget:thinking)>(.*)";

// A reasoning_content/reasoning provider field wins over tag-parsing the text, present-but-null included.
#[must_use]
pub fn extract_reasoning_content(message: &Message) -> (Option<String>, Option<Content>) {
    if let Some(value) = message.provider_specific_fields.get("reasoning_content") {
        return (value.as_str().map(str::to_owned), message.content.clone());
    }
    if let Some(value) = message.provider_specific_fields.get("reasoning") {
        return (value.as_str().map(str::to_owned), message.content.clone());
    }
    match &message.content {
        Some(Content::Text(text)) => {
            let (reasoning, content) = parse_content_for_reasoning(Some(text));
            (reasoning, content.map(Content::Text))
        }
        other => (None, other.clone()),
    }
}

#[must_use]
pub fn parse_content_for_reasoning(message_text: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(text) = message_text.filter(|text| !text.is_empty()) else {
        return (None, message_text.map(str::to_owned));
    };
    let Ok(pattern) = Regex::new(REASONING_TAG_PATTERN) else {
        return (None, Some(text.to_owned()));
    };
    let Ok(Some(captured)) = pattern.captures(text) else {
        return (None, Some(text.to_owned()));
    };
    let reasoning = captured.get(1).map(|matched| matched.as_str().to_owned());
    let remainder = captured
        .get(2)
        .map_or_else(String::new, |matched| matched.as_str().to_owned());
    (reasoning, Some(remainder))
}

mod tests;
