pub mod choices;
pub mod endpoint;
pub mod reply;
pub mod stream;
pub mod tool_calls;
pub mod wire;

mod tests;

use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::chat_choices::choices::{ChoicesError, transform_choices};
use crate::chat_choices::wire::{ReplyChoice, TransformedChoice};
use crate::openai_base_config::Object;

#[derive(Debug, Error)]
pub enum ChatChoicesError {
    #[error("the chat reply is not valid JSON")]
    Decode {
        #[source]
        source: Box<serde_json::Error>,
    },
    #[error(transparent)]
    Choices(Box<ChoicesError>),
}

impl From<ChoicesError> for ChatChoicesError {
    fn from(source: ChoicesError) -> Self {
        Self::Choices(Box::new(source))
    }
}

#[derive(Deserialize)]
struct ReplyEnvelope {
    choices: Vec<ReplyChoice>,
}

pub fn decode_chat_choices(
    bytes: &[u8],
    optional_params: Option<&Object>,
    json_mode: bool,
    identifier: fn() -> Uuid,
) -> Result<Vec<TransformedChoice>, ChatChoicesError> {
    let envelope: ReplyEnvelope =
        serde_json::from_slice(bytes).map_err(|source| ChatChoicesError::Decode {
            source: Box::new(source),
        })?;
    let choices = transform_choices(&envelope.choices, json_mode, optional_params, identifier)?;
    Ok(choices)
}
