mod input;
mod ops;
mod support;

use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = support::runtime()?;
        input
            .steps
            .iter()
            .map(|step| ops::execute(step, &runtime))
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array)
    }
}
