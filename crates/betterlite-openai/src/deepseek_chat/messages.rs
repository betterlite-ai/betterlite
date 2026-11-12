use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{
    Content, ContentPart, Message, convert_content_list_to_str, extract_search_results_text,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::chat_content::ChatContentError;

const IMAGE_URL: &str = "image_url";
const SEARCH_RESULTS: &str = "search_results";
const TEXT: &str = "text";
const URL: &str = "url";
const USER: &str = "user";

#[must_use]
pub fn forward_or_collapse(messages: &[Message], supports_vision: bool) -> Vec<Message> {
    let forward_images = supports_vision
        && messages
            .iter()
            .any(|message| matches!(&message.content, Some(Content::Parts(_))));
    messages
        .iter()
        .map(|message| forward_or_collapse_one(message, forward_images))
        .collect()
}

fn forward_or_collapse_one(message: &Message, forward_images: bool) -> Message {
    if forward_images && let Some(parts) = forwardable_parts(message) {
        return with_search_results_block(message, parts);
    }
    let collapsed = convert_content_list_to_str(message);
    let unchanged = match &message.content {
        Some(Content::Text(text)) => *text == collapsed,
        Some(Content::Parts(_)) | None => false,
    };
    if collapsed.is_empty() || unchanged {
        return message.clone();
    }
    let mut collapsed_message = message.clone();
    collapsed_message.content = Some(Content::Text(collapsed));
    collapsed_message
}

fn forwardable_parts(message: &Message) -> Option<&[ContentPart]> {
    if message.role != USER {
        return None;
    }
    let Some(Content::Parts(parts)) = &message.content else {
        return None;
    };
    let well_formed = parts.iter().all(is_forwardable_block);
    let has_image = parts.iter().any(|part| part.kind == IMAGE_URL);
    (well_formed && has_image).then_some(parts.as_slice())
}

fn is_forwardable_block(part: &ContentPart) -> bool {
    match part.kind.as_str() {
        IMAGE_URL => part
            .fields
            .get(IMAGE_URL)
            .is_some_and(image_url_is_forwardable),
        TEXT => part
            .fields
            .get(TEXT)
            .is_some_and(|value| value.as_str().is_some()),
        _ => false,
    }
}

fn image_url_is_forwardable(value: &Json) -> bool {
    match value {
        Json::Text(url) => !url.is_empty(),
        Json::Object(fields) => fields
            .get(URL)
            .and_then(Json::as_str)
            .is_some_and(|url| !url.is_empty()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => false,
    }
}

fn with_search_results_block(message: &Message, parts: &[ContentPart]) -> Message {
    let search_results = message.provider_specific_fields.get(SEARCH_RESULTS);
    let search_text = extract_search_results_text(search_results);
    if search_text.is_empty() {
        return message.clone();
    }
    let mut forwarded = message.clone();
    let mut content = parts.to_vec();
    content.push(ContentPart {
        kind: TEXT.to_owned(),
        fields: Object::from_iter([(TEXT.to_owned(), Json::Text(search_text))]),
    });
    forwarded.content = Some(Content::Parts(content));
    forwarded
        .provider_specific_fields
        .shift_remove(SEARCH_RESULTS);
    forwarded
}

pub(super) fn convert_messages<Source, Target>(
    messages: &[Source],
) -> Result<Vec<Target>, ChatContentError>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        let value = serde_json::to_value(message)
            .map_err(|source| ChatContentError::MessageShape { source })?;
        converted.push(
            serde_json::from_value(value)
                .map_err(|source| ChatContentError::MessageShape { source })?,
        );
    }
    Ok(converted)
}
