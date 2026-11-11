mod input;
mod logs;
mod ops;
mod tests;

use crate::helpers::{Helper, HelperResult};
use serde_json::json;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let factory = matches!(input, input::Input::CounterFactory(_));
        let calls = input
            .calls()
            .calls
            .iter()
            .map(|call| ops::run_call(call, factory))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"calls": calls}))
    }
}
