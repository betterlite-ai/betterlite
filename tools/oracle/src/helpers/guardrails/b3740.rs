use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod guardrails;
mod wire;

pub(crate) struct Op;

impl Helper for Op {
    type Input = wire::Input;

    fn run(input: &Self::Input) -> HelperResult {
        guardrails::b3740::run(input)
    }
}
