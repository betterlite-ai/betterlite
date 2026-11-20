use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod answers;
mod cases;
mod fakes;
mod fixtures;

use cases::Case;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    // boxed so every variant carries a pointer, well within the 3x variant_size_differences enforces
    HeaderMerge(Box<Case>),
    LoggingHook(Box<Case>),
    SuccessEvent(Box<Case>),
    FailureEvent(Box<Case>),
    Disconnect(Box<Case>),
    SuccessHook(Box<Case>),
    FailureHook(Box<Case>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::HeaderMerge(case) => Ok(answers::logging_answer(case, false)),
            Input::LoggingHook(case) => Ok(answers::logging_answer(case, true)),
            Input::SuccessEvent(case) => answers::success_event_answer(case),
            Input::FailureEvent(case) => answers::failure_event_answer(case),
            Input::Disconnect(case) => answers::disconnect_answer(case),
            Input::SuccessHook(case) => answers::success_hook_answer(case),
            Input::FailureHook(case) => answers::failure_hook_answer(case),
        }
    }
}
