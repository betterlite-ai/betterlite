use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod common;
mod supported;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Window(common::WindowSteps),
    ChatRequest(common::ChatRequestSteps),
    ExtraBody(common::ExtraBodySteps),
    ResponsesParams(common::ResponsesParamsSteps),
    Supported(supported::Steps),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Window(steps) => Ok(common::windows(steps)),
            Input::ChatRequest(steps) => common::chat_requests(steps),
            Input::ExtraBody(steps) => common::extra_bodies(steps),
            Input::ResponsesParams(steps) => common::responses_params(steps),
            Input::Supported(steps) => Ok(supported::run(steps)),
        }
    }
}
