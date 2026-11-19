mod input;
mod ops;
mod support;

use serde_json::Value;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = support::runtime()?;
        let steps = Value::Array(
            input
                .steps
                .iter()
                .map(|step| ops::execute(step, &runtime))
                .collect(),
        );
        Ok(steps)
    }
}
