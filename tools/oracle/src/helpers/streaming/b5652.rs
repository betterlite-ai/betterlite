use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod steps;

pub(crate) struct Op;

impl Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> HelperResult {
        steps::run(input)
    }
}
