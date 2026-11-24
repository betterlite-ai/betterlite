use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod ops;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
