use serde::Deserialize;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use envs::EnvsInput;
use names::NamesInput;
use wire::WireInput;

mod envs;
mod names;
mod specs;
mod tests;
mod wire;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Names(group) => names::run(group),
            Input::Envs(group) => envs::run(group),
            Input::Wire(group) => wire::run(group),
        }
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    Names(NamesInput),
    Envs(EnvsInput),
    Wire(WireInput),
}
