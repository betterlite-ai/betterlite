use crate::arguments::json::{Json, Object};

use super::{Content, ContentPart, Message, as_fields};

const IMAGE_SCAN_MAX_DEPTH: usize = 4;

#[must_use]
pub fn anthropic_image_source_url(source: &Object) -> Option<String> {
    match source.get("type").and_then(Json::as_str) {
        Some("base64") => {
            let media_type = source
                .get("media_type")
                .and_then(Json::as_str)
                .unwrap_or("image/jpeg");
            let data = source.get("data").and_then(Json::as_str).unwrap_or("");
            (!data.is_empty()).then(|| format!("data:{media_type};base64,{data}"))
        }
        // LiteLLM's "url" branch returns "" rather than None when the url key is missing or not a string.
        Some("url") => Some(
            source
                .get("url")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        _ => None,
    }
}

#[must_use]
pub fn as_openai_image_part(part: &ContentPart) -> Option<ContentPart> {
    let url = image_part_url(part)?;
    let image_url = Object::from_iter([("url".to_owned(), Json::Text(url))]);
    Some(ContentPart {
        kind: "image_url".to_owned(),
        fields: Object::from_iter([("image_url".to_owned(), Json::Object(image_url))]),
    })
}

#[must_use]
pub fn request_contains_image_content(messages: &[Message]) -> bool {
    for message in messages {
        if matches!(&message.content, Some(Content::Parts(parts)) if content_parts_contain_image(parts))
        {
            return true;
        }
    }
    false
}

#[must_use]
pub fn message_has_audio_or_image(message: &Message) -> bool {
    let Some(Content::Parts(parts)) = &message.content else {
        return false;
    };
    parts
        .iter()
        .any(|part| part.kind == "image_url" || part.kind == "input_audio")
}

fn content_parts_contain_image(parts: &[ContentPart]) -> bool {
    let mut frontier = parts
        .iter()
        .map(|part| Json::Object(as_fields(part)))
        .collect::<Vec<_>>();
    for _ in 0..IMAGE_SCAN_MAX_DEPTH {
        if frontier.iter().any(part_is_image) {
            return true;
        }
        frontier = frontier
            .iter()
            .filter_map(nested_content)
            .flatten()
            .collect();
        if frontier.is_empty() {
            return false;
        }
    }
    false
}

fn part_is_image(part: &Json) -> bool {
    let Json::Object(fields) = part else {
        return false;
    };
    fields
        .get("type")
        .and_then(Json::as_str)
        .is_some_and(|kind| matches!(kind, "image_url" | "input_image" | "image"))
}

fn nested_content(part: &Json) -> Option<Vec<Json>> {
    let Json::Object(fields) = part else {
        return None;
    };
    match fields.get("content") {
        Some(Json::List(items)) => Some(items.clone()),
        _ => None,
    }
}

fn image_part_url(part: &ContentPart) -> Option<String> {
    if part.kind == "image_url" {
        return chat_completions_image_url(&part.fields);
    }
    if part.kind == "input_image" {
        return part
            .fields
            .get("image_url")
            .and_then(Json::as_str)
            .map(str::to_owned);
    }
    if part.kind == "image" {
        return part
            .fields
            .get("source")
            .and_then(Json::as_object)
            .and_then(anthropic_image_source_url);
    }
    None
}

fn chat_completions_image_url(fields: &Object) -> Option<String> {
    match fields.get("image_url") {
        Some(Json::Text(url)) => Some(url.clone()),
        Some(Json::Object(url)) => url.get("url").and_then(Json::as_str).map(str::to_owned),
        _ => None,
    }
}

mod tests;
