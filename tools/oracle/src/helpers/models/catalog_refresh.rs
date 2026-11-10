mod input;
mod scenario;
mod scripting;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        scenario::run(input)
    }
}

pub(super) fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
