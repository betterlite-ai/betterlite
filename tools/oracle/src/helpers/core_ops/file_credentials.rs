mod answers;
mod catalog;
mod input;

use std::fmt::Display;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::Value;

use answers::answer;
use input::Input;

fn failure<Message>(message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer(call)?);
        }
        Ok(Value::Array(answers))
    }
}
