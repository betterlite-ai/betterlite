use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod entities;
mod gauges;
mod input;
mod lists;
mod output;
mod probe;
mod provider;
mod run;
mod support;
mod team_members;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        run::run(input)
    }
}
