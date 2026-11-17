use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use crate::converse_messages::error::ConverseMessageError;
use crate::model_routes::ParamMap;

const GUARDRAIL_FIELD: &str = "guardrailConfig";
const GUARDED_TEXT: &str = "guarded_text";
const TEXT_FIELD: &str = "text";
const TEXT_PART: &str = "text";

// The guardrail only sees the trailing user turn, so LiteLLM converts exactly that suffix.
pub fn guard_trailing_user_turns(
    messages: &[Message],
    optional_params: &ParamMap,
) -> Result<Vec<Message>, ConverseMessageError> {
    if !optional_params.contains_key(GUARDRAIL_FIELD) {
        return Ok(messages.to_vec());
    }
    let Some(start) = user_suffix_start(messages) else {
        return Ok(messages.to_vec());
    };
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            if index < start {
                Ok(message.clone())
            } else {
                guarded_message(message, index)
            }
        })
        .collect()
}

fn user_suffix_start(messages: &[Message]) -> Option<usize> {
    let start = messages
        .iter()
        .rposition(|message| message.role != "user")
        .map_or(0, |last_other| last_other.saturating_add(1));
    if start == messages.len() {
        return None;
    }
    Some(start)
}

fn guarded_message(message: &Message, index: usize) -> Result<Message, ConverseMessageError> {
    let mut converted = message.clone();
    match message.content.as_ref() {
        Some(Content::Text(text)) => {
            converted.content = Some(Content::Parts(vec![guarded_block(Json::Text(
                text.clone(),
            ))]));
        }
        Some(Content::Parts(parts)) => {
            if parts.iter().any(|part| part.kind == GUARDED_TEXT) {
                return Ok(converted);
            }
            let mut guarded_parts = Vec::with_capacity(parts.len());
            for part in parts {
                if part.kind == TEXT_PART {
                    let Some(text) = part.fields.get(TEXT_FIELD) else {
                        return Err(ConverseMessageError::TextPartWithoutText { message: index });
                    };
                    guarded_parts.push(guarded_block(text.clone()));
                } else {
                    guarded_parts.push(part.clone());
                }
            }
            converted.content = Some(Content::Parts(guarded_parts));
        }
        None => {
            converted.content = Some(Content::Parts(Vec::new()));
        }
    }
    Ok(converted)
}

// Bedrock's guarded_text part holds the text alone; cache_control and any other part field stay behind.
fn guarded_block(text: Json) -> ContentPart {
    ContentPart {
        kind: GUARDED_TEXT.to_owned(),
        fields: Object::from_iter([(TEXT_FIELD.to_owned(), text)]),
    }
}
