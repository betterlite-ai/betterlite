mod fixture;
mod reports;
mod steps;
mod tests;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl Helper for Op {
    type Input = steps::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error.to_string()))?;
        let reports: Vec<Value> = input
            .steps
            .iter()
            .cloned()
            .map(|step| steps::run_step(&runtime, &step))
            .collect();
        Ok(json!({ "steps": reports }))
    }
}
