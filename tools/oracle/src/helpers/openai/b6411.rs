use betterlite_openai::sail_common::{SailSettings, chat_request_for_sail, extra_body_for_sail};

// ast-grep-ignore: no-long-names
use betterlite_openai::sail_common::responses::responses_params_with_completion_window as params_with_window;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

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
            Input::Window(steps) => common::windows(steps),
            Input::ChatRequest(steps) => common::records(steps.steps.iter().map(|step| {
                let settings = SailSettings {
                    global_drop_params: step.global_drop_params,
                };
                chat_request_for_sail(&step.request, &steps.model, step.drop_params, &settings)
            })),
            Input::ExtraBody(steps) => common::records(steps.steps.iter().map(|step| {
                let settings = SailSettings {
                    global_drop_params: step.global_drop_params,
                };
                extra_body_for_sail(
                    &step.extra_body,
                    Some(&step.request_metadata),
                    &steps.model,
                    step.drop_params,
                    &settings,
                )
            })),
            Input::ResponsesParams(steps) => common::records(steps.steps.iter().map(|step| {
                let settings = SailSettings {
                    global_drop_params: step.global_drop_params,
                };
                params_with_window(&step.params, &steps.model, step.drop_params, &settings)
            })),
            Input::Supported(steps) => Ok(supported::run(steps)),
        }
    }
}
