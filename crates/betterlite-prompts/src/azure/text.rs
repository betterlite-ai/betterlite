use crate::arguments::json::Json;
use crate::azure::error::AzureError;
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::message_content::{Content, ContentPart, Message};

const TEXT: &str = "text";

pub fn azure_text_pt(messages: &[Message]) -> Result<String, AzureError> {
    let mut prompt = String::new();
    for (index, message) in messages.iter().enumerate() {
        match &message.content {
            Some(Content::Text(text)) => prompt.push_str(text),
            Some(Content::Parts(parts)) => append_text_parts(&mut prompt, parts, index)?,
            None => {}
        }
    }
    Ok(prompt)
}

fn append_text_parts(
    prompt: &mut String,
    parts: &[ContentPart],
    index: usize,
) -> Result<(), AzureError> {
    if parts.len() > MAX_MESSAGE_PARTS {
        return Err(AzureError::TooManyParts {
            limit: MAX_MESSAGE_PARTS,
        });
    }
    for (part_index, part) in parts.iter().enumerate() {
        if part.kind != TEXT {
            continue;
        }
        let Some(Json::Text(text)) = part.fields.get(TEXT) else {
            return Err(AzureError::Text {
                index,
                part: part_index,
            });
        };
        prompt.push_str(text);
    }
    Ok(())
}
