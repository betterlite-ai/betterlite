mod fixture;
mod reports;
mod steps;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = steps::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let reports: Vec<Value> = input.steps.iter().cloned().map(steps::run_step).collect();
        Ok(json!({ "steps": reports }))
    }
}
