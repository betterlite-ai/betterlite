use betterlite_core::api::messages_responses_input::error::MessagesResponsesInputError;
use betterlite_core::api::messages_responses_input::{
    MessagesResponsesInputInput, messages_responses_input,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

// Each failure names the exception class the pinned adapter raises on the same input.
fn failure(error: &MessagesResponsesInputError) -> Box<HelperError> {
    let class = match error {
        MessagesResponsesInputError::ToolInput { .. }
        | MessagesResponsesInputError::Usage { .. } => "TypeError",
        MessagesResponsesInputError::UsageRecord { .. } => "ValidationError",
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
    type Input = MessagesResponsesInputInput;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = messages_responses_input(input).map_err(|error| failure(&error))?;
        Ok(serde_json::to_value(answer)?)
    }
}
