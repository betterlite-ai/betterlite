use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

macro_rules! helper_error {
    ($class:expr, $message:expr) => {
        Box::new(HelperError {
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
    type Input = parse::Input;

    fn run(input: &Self::Input) -> HelperResult {
        execution::run(input)
    }
}
