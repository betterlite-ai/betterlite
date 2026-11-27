use serde_json::Value;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod driver;
mod input;
mod tests;

use driver::observe;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers: Vec<Value> = input
            .calls
            .iter()
            .map(|call| observe(call, &input.model_cost))
            .collect();
        Ok(Value::Array(answers))
    }
}
