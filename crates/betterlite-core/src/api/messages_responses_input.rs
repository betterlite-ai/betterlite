use betterlite_openai::api::responses_objects::ResponseApiUsage;
use serde::{Deserialize, Serialize};
use serde_with::{As, DefaultOnNull};

use crate::api::messages_chat_output::output::UsageProjection;
use error::MessagesResponsesInputError;
use items::InputItem;
use records::MessageRecord;

pub mod error;
pub mod items;
pub mod records;
pub mod thinking;
pub mod translate;
pub mod usage;

mod media;
mod system;
mod tests;
mod tool_results;

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesResponsesInputInput {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub messages: Vec<MessageRecord>,
    #[serde(default)]
    pub usage: Option<ResponseApiUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessagesResponsesInputResult {
    pub input: Vec<InputItem>,
    pub usage: UsageProjection,
}

pub fn messages_responses_input(
    input: &MessagesResponsesInputInput,
) -> Result<MessagesResponsesInputResult, MessagesResponsesInputError> {
    Ok(MessagesResponsesInputResult {
        input: translate::messages_to_input(&input.messages)?,
        usage: usage::anthropic_usage(input.usage.as_ref())?,
    })
}
