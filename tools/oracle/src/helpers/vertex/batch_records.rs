use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod ops;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = ops::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}
