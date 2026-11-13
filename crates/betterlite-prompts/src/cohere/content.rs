use crate::arguments::json::Json;
use crate::cohere::error::CohereError;
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::message_content::{Content, ContentPart};

const TEXT: &str = "text";

pub(crate) fn content_text(content: Option<&Content>, index: usize) -> Result<String, CohereError> {
    match content {
        Some(Content::Text(text)) => Ok(text.clone()),
        Some(Content::Parts(parts)) => parts_text(parts, index),
        None => Err(CohereError::MissingContent { index }),
    }
}

pub(crate) fn optional_content_text(
    content: Option<&Content>,
    index: usize,
) -> Result<String, CohereError> {
    match content {
        Some(Content::Text(text)) => Ok(text.clone()),
        Some(Content::Parts(parts)) => parts_text(parts, index),
        None => Ok(String::new()),
    }
}

pub(crate) fn parts_text(parts: &[ContentPart], index: usize) -> Result<String, CohereError> {
    if parts.len() > MAX_MESSAGE_PARTS {
        return Err(CohereError::TooManyParts {
            limit: MAX_MESSAGE_PARTS,
        });
    }
    let mut text = String::new();
    for (part_index, part) in parts.iter().enumerate() {
        if part.kind != TEXT {
            continue;
        }
        let Some(Json::Text(value)) = part.fields.get(TEXT) else {
            return Err(CohereError::TextPart {
                index,
                part: part_index,
            });
        };
        text.push_str(value);
    }
    Ok(text)
}
