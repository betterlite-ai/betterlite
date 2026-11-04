use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::messages::{detect_first_expected_role, get_completion_messages};
use serde_json::{Value, to_value};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let operation = input
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let messages = input
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let result = match operation {
            "detect_first_expected_role" => to_value(detect_first_expected_role(&messages)),
            "get_completion_messages" => {
                let assistant = input.get("assistant_continue_message");
                let user = input.get("user_continue_message");
                let ensure = input
                    .get("ensure_alternating_roles")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                to_value(get_completion_messages(&messages, assistant, user, ensure))
            }
            _ => {
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
