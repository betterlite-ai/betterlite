use crate::helpers::{Helper, HelperResult};
use serde_json::Value;

macro_rules! helper_error {
    ($class:expr, $message:expr) => {
        Box::new(crate::helpers::HelperError {
            class: $class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: $message,
        })
    };
}

mod execution;
mod parse;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        execution::run(input)
    }
}
