use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod entities;
mod input;
mod lookups;
mod output;
mod probe;
mod run;
mod support;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        run::run(input)
    }
}
