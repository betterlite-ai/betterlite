mod fixture;
mod input;
mod ops;
mod replies;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        ops::run(input)
    }
}

type Step<Case> = fn(&Case) -> Result<Value, Box<HelperError>>;

// every family answers its cases as steps in the input's order, and supplies the one step it takes
fn steps<Case>(items: &[Case], step: Step<Case>) -> Result<Value, Box<HelperError>> {
    let values: Vec<Value> = items.iter().map(step).collect::<Result<_, _>>()?;
    Ok(json!({"steps": values}))
}
