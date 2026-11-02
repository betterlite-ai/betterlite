use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, ContentPart, Message};
use crate::system_placement::cache_control::{CacheControl, cache_control, cache_control_block};

pub const CONVERTED_SYSTEM_NOTE: &str = "Operator note (not from the user): the following was \
originally a mid-conversation system-role reminder.";

pub struct TextPart {
    pub text: String,
    pub cache_control: Option<CacheControl>,
}

// Anthropic rejects empty text blocks; a cache_control on the message itself belongs to the block built from string content.
#[must_use]
pub fn text_parts(message: &Message) -> Vec<TextPart> {
    match &message.content {
        Some(Content::Text(text)) if !text.is_empty() => vec![TextPart {
            text: text.clone(),
            cache_control: cache_control(&message.provider_specific_fields),
        }],
        Some(Content::Parts(parts)) => parts.iter().filter_map(text_part).collect(),
        Some(Content::Text(_)) | None => Vec::new(),
    }
}

fn text_part(part: &ContentPart) -> Option<TextPart> {
    if part.kind != "text" {
        return None;
    }
    let text = part.fields.get("text").and_then(Json::as_str)?;
    (!text.is_empty()).then(|| TextPart {
        text: text.to_owned(),
        cache_control: cache_control(&part.fields),
    })
}

#[must_use]
pub fn text_block(part: &TextPart) -> ContentPart {
    let mut fields = Object::from_iter([("text".to_owned(), Json::Text(part.text.clone()))]);
    if let Some(block_cache_control) = &part.cache_control {
        fields.insert(
            "cache_control".to_owned(),
            cache_control_block(block_cache_control),
        );
    }
    ContentPart {
        kind: "text".to_owned(),
        fields,
    }
}

fn note_block() -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: Object::from_iter([(
            "text".to_owned(),
            Json::Text(CONVERTED_SYSTEM_NOTE.to_owned()),
        )]),
    }
}

// A system message re-rolled as a user turn, prefixed with the operator note so the model can tell the instruction apart from the user's own words.
#[must_use]
pub fn system_message_as_user(message: &Message) -> Message {
    let mut content = vec![note_block()];
    content.extend(text_parts(message).iter().map(text_block));
    Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(content)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

#[must_use]
pub fn merged_system_message(run: &[Message]) -> Option<Message> {
    let parts: Vec<ContentPart> = run
        .iter()
        .flat_map(text_parts)
        .map(|part| text_block(&part))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(Message {
        role: "system".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    })
}

#[must_use]
pub fn converted_user_turns(run: &[Message]) -> Vec<Message> {
    run.iter()
        .filter(|message| !text_parts(message).is_empty())
        .map(system_message_as_user)
        .collect()
}

mod tests;
