use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

use capabilities::CapabilityInput;
use conversion::ConversionInput;
use errors::ErrorInput;

mod capabilities;
mod conversion;
mod errors;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Capabilities(group) => capabilities::run(group),
            Input::Conversion(group) => conversion::run(group),
            Input::Errors(group) => Ok(errors::run(group)),
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
    Capabilities(CapabilityInput),
    Conversion(ConversionInput),
    Errors(ErrorInput),
}
