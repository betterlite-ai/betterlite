use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::messages::{detect_first_expected_role, get_completion_messages};
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    operation: String,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    assistant_continue_message: Option<Message>,
    #[serde(default)]
    user_continue_message: Option<Message>,
    ensure_alternating_roles: Option<bool>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let result = match input.operation.as_str() {
            "detect_first_expected_role" => to_value(detect_first_expected_role(&input.messages)),
            "get_completion_messages" => to_value(get_completion_messages(
                &input.messages,
                input.assistant_continue_message.as_ref(),
                input.user_continue_message.as_ref(),
                input.ensure_alternating_roles.unwrap_or(true),
            )),
            operation => {
                return Err(classified_error(format!(
                    "unsupported operation {operation}"
                )));
            }
        };
        result.map_err(|error| classified_error(error.to_string()))
    }
}

fn classified_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
