use betterlite_provider::ollama_chat::config::OllamaChatConfig;
use serde::Deserialize;

use self::script::{ParamCase, ReplyCase, RequestCase, ValidateCase};
use crate::helpers::Helper;
use crate::helpers::HelperResult;

mod ops;
mod script;

#[derive(Deserialize)]
pub(crate) struct ParamsInput {
    #[serde(default)]
    config: OllamaChatConfig,
    cases: Vec<ParamCase>,
    #[serde(default)]
    validate: Vec<ValidateCase>,
    #[serde(default)]
    urls: Vec<Option<String>>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Params(Box<ParamsInput>),
    Request { requests: Vec<RequestCase> },
    Reply { replies: Vec<ReplyCase> },
    Stream { lines: Vec<String> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Params(params) => Ok(ops::params(
                &params.config,
                &params.cases,
                &params.validate,
                &params.urls,
            )),
            Input::Request { requests } => Ok(ops::requests(requests)),
            Input::Reply { replies } => ops::replies(replies),
            Input::Stream { lines } => Ok(ops::stream(lines)),
        }
    }
}
