mod input;
mod ops;
mod tests;

use crate::helpers::{Helper, HelperResult};
use serde_json::json;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Calls;

    fn run(input: &Self::Input) -> HelperResult {
        let calls = input
            .calls
            .iter()
            .map(ops::run_call)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"calls": calls}))
    }
}
