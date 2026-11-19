mod fixtures;
mod input;
mod reports;
mod steps;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values: Vec<Value> = input
            .steps
            .iter()
            .map(steps::run)
            .collect::<Result<_, _>>()?;
        Ok(json!({ "steps": values }))
    }
}
