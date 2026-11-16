use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::json;

mod answers;
mod input;
mod secrets;
mod tests;

use answers::run_calls;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!({"calls": run_calls(input)?}))
    }
}
