use betterlite_openai::api::responses_objects::ResponseApiUsage;

use crate::api::messages_chat_output::output::UsageProjection;
use crate::api::messages_chat_output::records::UsageRecord;
use crate::api::messages_chat_output::usage::project_usage;
use crate::api::responses_usage::{UsageInput, normalize_and_project};

use super::error::MessagesResponsesInputError;

// A Responses usage becomes the Anthropic usage its counts translate to; no usage reports two zero counters.
pub fn anthropic_usage(
    raw_usage: Option<&ResponseApiUsage>,
) -> Result<UsageProjection, MessagesResponsesInputError> {
    let input = match raw_usage {
        None => UsageInput::Absent,
        Some(usage) => UsageInput::Response(usage),
    };
    let chat_usage = normalize_and_project(input)
        .map_err(|source| MessagesResponsesInputError::Usage { source })?;
    let record: UsageRecord = serde_json::from_value(chat_usage)
        .map_err(|source| MessagesResponsesInputError::UsageRecord { source })?;
    Ok(project_usage(&record))
}
