use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::image::message_has_audio_or_image;
use betterlite_prompts::message_content::{Content, Message, convert_content_list_to_str};
use betterlite_prompts::message_fields::nested_filter::filter_value_from_dict;
use thiserror::Error;

pub const NON_OPENAI_SPEC_MESSAGE_FIELDS: [&str; 4] = [
    "thinking_blocks",
    "reasoning_content",
    "provider_specific_fields",
    "cache_control",
];

#[derive(Debug, Error)]
pub enum MessagesError {
    #[error("a chat message is not a JSON object")]
    NotAnObject,
    #[error("a chat message could not be read")]
    Json(#[from] serde_json::Error),
}

pub fn transform_messages(messages: &[Message]) -> Result<Vec<Message>, MessagesError> {
    let mut transformed = Vec::with_capacity(messages.len());
    for message in messages {
        let mut stripped = strip_non_openai_fields(message)?;
        if !message_has_audio_or_image(&stripped) {
            let text = convert_content_list_to_str(&stripped);
            if !text.is_empty() {
                stripped.content = Some(Content::Text(text));
            }
        }
        transformed.push(stripped);
    }
    Ok(transformed)
}

fn strip_non_openai_fields(message: &Message) -> Result<Message, MessagesError> {
    let value = serde_json::to_value(message)?;
    let Json::Object(mut fields) = serde_json::from_value::<Json>(value)? else {
        return Err(MessagesError::NotAnObject);
    };
    for field in NON_OPENAI_SPEC_MESSAGE_FIELDS {
        fields = filter_value_from_dict(&fields, field, 0);
    }
    Ok(serde_json::from_value(serde_json::to_value(
        Json::Object(fields),
    )?)?)
}
