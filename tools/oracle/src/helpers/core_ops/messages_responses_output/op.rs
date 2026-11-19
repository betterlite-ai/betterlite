use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_responses_output::error::MessagesResponsesOutputError;
use betterlite_core::api::messages_responses_output::service::{
    MessagesResponsesOutpuInput, messages_responses_output,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use super::Op;

// Each failure names the exception class the pinned adapter raises on the same input.
fn failure(error: &MessagesResponsesOutputError) -> Box<HelperError> {
    let class = match error {
        MessagesResponsesOutputError::Json { .. } => "JSONDecodeError",
        MessagesResponsesOutputError::ArgumentsNotObject
        | MessagesResponsesOutputError::Usage { .. } => "ValidationError",
        MessagesResponsesOutputError::Output { .. }
        | MessagesResponsesOutputError::Messages { .. }
        | MessagesResponsesOutputError::Limit(_)
        | MessagesResponsesOutputError::SessionKey { .. }
        | MessagesResponsesOutputError::ContextThreshold
        | MessagesResponsesOutputError::Effort { .. } => "ValueError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Helper for Op {
    type Input = MessagesResponsesOutpuInput;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = messages_responses_output(input, &ApiLimits::default())
            .map_err(|error| failure(&error))?;
        Ok(serde_json::to_value(answer)?)
    }
}
