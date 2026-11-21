use betterlite_foundry::rejected_fields::MAX_ATTEMPTS;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod reports;
mod tests;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answer = Map::new();
        if let Some(calls) = &input.triggers {
            let values: Vec<Value> = calls.iter().map(reports::trigger_report).collect();
            answer.insert(String::from("triggers"), Value::Array(values));
        }
        if let Some(calls) = &input.repairs {
            let values: Vec<Value> = calls.iter().map(reports::repair_report).collect();
            answer.insert(String::from("repairs"), Value::Array(values));
        }
        if input.bounds {
            answer.insert(String::from("bounds"), json!({"max_retry": MAX_ATTEMPTS}));
        }
        Ok(Value::Object(answer))
    }
}
