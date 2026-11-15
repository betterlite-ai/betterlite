use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

use input::Input;
use views::run_call;

mod input;
mod tests;
mod views;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call)?);
        }
        Ok(Value::Array(results))
    }
}
