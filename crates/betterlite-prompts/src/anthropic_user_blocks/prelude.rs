use thiserror::Error;

use crate::anthropic_history::HistoryError;
use crate::anthropic_history::text::sanitize_empty_text_content;
use crate::anthropic_history::tools::{MAX_HISTORY_MESSAGES, sanitize_messages};
use crate::arguments::json::{Json, Object};
use crate::media::limits::MAX_MESSAGE_PARTS;

const CONTINUE_TEXT: &str = "Please continue.";
const CONTENT: &str = "content";
const ROLE: &str = "role";
const USER: &str = "user";

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(
        "Anthropic needs at least one non-system message; set modify_params to add the dummy user message"
    )]
    NoUserMessages,
    #[error("messages exceeds the maximum of {limit}")]
    TooManyMessages { limit: usize },
    #[error("a content list exceeds the maximum of {limit} parts")]
    TooManyParts { limit: usize },
    #[error("message at index {index} is not an object")]
    MessageNotObject { index: usize },
}

#[derive(Debug)]
pub struct Prepared {
    pub messages: Vec<Json>,
    pub force_base64: bool,
}

pub fn prepare(
    messages: &[Json],
    model: &str,
    llm_provider: &str,
    modify_params: bool,
) -> Result<Prepared, PrepareError> {
    check_bounds(messages)?;
    let sanitized = sanitize_messages(messages, modify_params)?;
    let mut prepared: Vec<Json> = sanitized.iter().map(sanitize_empty_text_content).collect();
    if prepared.is_empty() {
        if !modify_params {
            return Err(PrepareError::NoUserMessages);
        }
        prepared.push(dummy_message());
    }
    Ok(Prepared {
        messages: prepared,
        force_base64: model.to_lowercase().starts_with("invoke/")
            || llm_provider.starts_with("vertex_ai"),
    })
}

fn check_bounds(messages: &[Json]) -> Result<(), PrepareError> {
    if messages.len() > MAX_HISTORY_MESSAGES {
        return Err(PrepareError::TooManyMessages {
            limit: MAX_HISTORY_MESSAGES,
        });
    }
    for (index, message) in messages.iter().enumerate() {
        let Some(fields) = message.as_object() else {
            return Err(PrepareError::MessageNotObject { index });
        };
        if let Some(Json::List(parts)) = fields.get(CONTENT)
            && parts.len() > MAX_MESSAGE_PARTS
        {
            return Err(PrepareError::TooManyParts {
                limit: MAX_MESSAGE_PARTS,
            });
        }
    }
    Ok(())
}

fn dummy_message() -> Json {
    Json::Object(Object::from_iter([
        (ROLE.to_owned(), Json::Text(USER.to_owned())),
        (CONTENT.to_owned(), Json::Text(CONTINUE_TEXT.to_owned())),
    ]))
}
