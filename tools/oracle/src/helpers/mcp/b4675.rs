mod answers;
mod input;
mod report;
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
        let mut reports = Vec::new();
        let mut captured: Vec<Option<String>> = Vec::new();
        for step in &input.steps {
            let (report, cookie) = scenario.report(step, &captured);
            reports.push(report);
            captured.push(cookie);
        }
        Ok(Value::Array(reports))
    }
}
