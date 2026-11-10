mod input;
mod ops;
mod tests;

use serde_json::json;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let steps = input.calls.iter().map(ops::run_call).collect::<Vec<_>>();
        Ok(json!({"steps": steps}))
    }
}
