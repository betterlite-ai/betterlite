mod cases;
mod input;
mod ops;
mod tests;
use crate::helpers::{Helper, HelperResult};
use serde_json::json;
pub(crate) struct Op;
impl Helper for Op {
    type Input = input::Input;
    fn run(input: &Self::Input) -> HelperResult {
        let steps = input.calls.iter().map(ops::run_call).collect::<Vec<_>>();
        Ok(json!({"steps": steps}))
    }
}
