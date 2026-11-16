mod handler;
mod input;
mod output;
mod run;
mod scenario;
mod state;
mod support;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use std::fmt::Display;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        run::run(input)
    }
}

pub(crate) fn value_error<E>(failure: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: failure.to_string(),
    })
}
