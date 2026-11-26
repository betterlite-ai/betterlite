use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::Value;

mod driver;
mod fixture;
mod input;
mod tests;

use driver::observe;
use fixture::Catalog;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let catalog = Catalog::new().or_config_error()?;
        let answers: Vec<Value> = input
            .calls
            .iter()
            .map(|call| observe(call, &input.model_cost, &catalog))
            .collect();
        Ok(Value::Array(answers))
    }
}
