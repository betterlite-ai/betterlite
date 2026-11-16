mod fixture;
mod reports;
mod steps;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = steps::Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(steps::run_input(input))
    }
}
