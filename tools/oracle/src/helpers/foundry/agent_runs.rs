use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Map, Value};
use tokio::runtime::Builder;

mod dispatches;
#[path = "agent_runs/fixtures/harness.rs"]
mod harness;
mod input;
mod runs;
mod tests;
mod views;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answer = Map::new();
        if let Some(calls) = &input.runs {
            answer.insert(String::from("runs"), runs::runs(calls, &runtime)?);
        }
        if let Some(calls) = &input.dispatches {
            answer.insert(
                String::from("dispatches"),
                dispatches::dispatches(calls, &runtime)?,
            );
        }
        Ok(Value::Object(answer))
    }
}
