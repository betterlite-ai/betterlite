use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixtures;
mod input;
mod output;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(output::probe(input.case())?)
    }
}
