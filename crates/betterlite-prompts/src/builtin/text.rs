use super::{Content, Message, PromptError};
use betterlite_logging::analytics::python;

pub(super) fn strict(message: &Message, index: usize) -> Result<&str, PromptError> {
    match &message.content {
        Content::Text(text) => Ok(text),
        Content::Parts(_) | Content::Null => Err(PromptError::Text { index }),
    }
}

pub(super) fn joined(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Null => String::new(),
        Content::Parts(parts) => parts.iter().filter_map(|part| part.text("text")).collect(),
    }
}

pub(super) fn display(content: &Content) -> Result<String, PromptError> {
    match content {
        Content::Text(text) => Ok(text.clone()),
        Content::Null => Ok("None".to_owned()),
        Content::Parts(parts) => Ok(python::repr(parts)?),
    }
}
