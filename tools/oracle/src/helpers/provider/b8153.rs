mod info;
mod params;
mod passthrough;
mod reply;
mod request;

use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};
use info::{InfoInput, model_info_values};
use params::{ParamsInput, params_value};
use passthrough::{PassInput, passthrough_values};
use reply::{ReplyCases, reply_values};
use request::{RequestInput, request_values};

const FIXED_NOW: i64 = 1_700_000_000;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    VllmModelInfo(InfoInput),
    VllmPassthrough(PassInput),
    PetalsParams(ParamsInput),
    PetalsRequest(RequestInput),
    PetalsReply(ReplyCases),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::VllmModelInfo(info) => Ok(model_info_values(info)),
            Input::VllmPassthrough(pass) => Ok(passthrough_values(pass)),
            Input::PetalsParams(params) => Ok(params_value(params)),
            Input::PetalsRequest(request) => request_values(request),
            Input::PetalsReply(replies) => reply_values(replies),
        }
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}
