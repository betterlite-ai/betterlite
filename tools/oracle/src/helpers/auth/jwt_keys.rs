use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fakes;
mod input;
mod scenario;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        scenario::answer(&input.scenarios)
    }
}
