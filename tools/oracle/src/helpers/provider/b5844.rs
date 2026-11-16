use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod ops;
mod project;
mod support;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::output(input)
    }
}
