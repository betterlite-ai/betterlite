use betterlite_core::api::responses_parameters::error::{
    ResponsesParametersError, UNSUPPORTED_PARAMS_STATUS,
};
use betterlite_core::api::responses_parameters::forwarded_headers::merge_forwarded_headers;
use serde::Deserialize;
use serde_json::{Value, json};

use self::reports::drop_flags;
use crate::helpers::{Helper, HelperError, HelperResult};

mod optional;
mod reports;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Optional(optional::OptionalInput),
    Requested(reports::RequestedInput),
    Prompt(reports::PromptInput),
    ForwardedHeaders(reports::ForwardedInput),
    McpHeaders(reports::McpInput),
    DropFlags(reports::DropFlagsInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Optional(fields) => each(&fields.steps, optional::step),
            Input::Requested(fields) => each(&fields.steps, reports::requested_step),
            Input::Prompt(fields) => each(&fields.steps, reports::prompt_step),
            Input::ForwardedHeaders(fields) => each(&fields.steps, |step| {
                Ok(json!(merge_forwarded_headers(
                    step.extra.clone(),
                    step.client.clone()
                )))
            }),
            Input::McpHeaders(fields) => each(&fields.steps, reports::mcp_step),
            Input::DropFlags(fields) => each(&fields.steps, |step| Ok(drop_flags(step))),
        }
    }
}

fn each<Step, Run>(steps: &[Step], run: Run) -> HelperResult
where
    Run: Fn(&Step) -> Result<Value, Box<HelperError>>,
{
    steps
        .iter()
        .map(run)
        .collect::<Result<Vec<Value>, Box<HelperError>>>()
        .map(Value::Array)
}

pub(super) fn helper_error(error: &ResponsesParametersError) -> Box<HelperError> {
    let status = matches!(error, ResponsesParametersError::UnsupportedParams { .. })
        .then_some(UNSUPPORTED_PARAMS_STATUS);
    Box::new(HelperError {
        class: "ResponsesParametersError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
