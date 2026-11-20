mod answers;
mod scripted;
mod values;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) type HelperFailure = Box<HelperError>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = scripted::Input;

    fn run(input: &Self::Input) -> HelperResult {
        answers::drive(input)
    }
}

// a raised Python exception compares with null type, param and code, so a native failure carries the same three fields
fn raised(class: &str, message: String) -> HelperFailure {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
