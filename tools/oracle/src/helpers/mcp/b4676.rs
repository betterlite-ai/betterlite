mod input;
mod normalize;
mod scenario;
mod support;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::Value;

use self::input::Input;
use self::scenario::Scenario;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let scenario = Scenario::new(input)?;
        Ok(Value::Array(scenario.reports()?))
    }
}
