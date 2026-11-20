mod driver;
mod input;
mod provider;
mod tests;
mod transport;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) type Failure = Box<HelperError>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        driver::drive(input)
    }
}

pub(crate) fn failure(class: &str, message: String) -> Failure {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
