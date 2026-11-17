mod input;
mod perform;
mod recorder;
mod tests;

use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let case = &input.pipeline;
        let sync = perform::perform(case, perform::Mode::Sync)?;
        let asynced = perform::perform(case, perform::Mode::Async)?;
        Ok(json!({"sync": sync, "async": asynced}))
    }
}
