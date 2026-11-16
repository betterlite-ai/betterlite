mod input;
mod ops;

use serde_json::Value;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .steps
            .iter()
            .map(ops::execute)
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
