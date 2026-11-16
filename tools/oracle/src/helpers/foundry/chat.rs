use betterlite_foundry::chat::fields::required_params;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod input;
mod reports;
mod tests;

use fixture::Installed;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut installed = Installed::new(input);
        let mut answer = Map::new();
        if let Some(calls) = &input.urls {
            let values = calls.iter().map(reports::url_report).collect();
            answer.insert(String::from("urls"), Value::Array(values));
        }
        if let Some(models) = &input.supported {
            let values = models
                .iter()
                .map(|model| reports::supported_report(&runtime, &mut installed, model))
                .collect();
            answer.insert(String::from("supported"), Value::Array(values));
        }
        if let Some(calls) = &input.maps {
            let values = calls
                .iter()
                .map(|call| reports::map_report(&runtime, &mut installed, call))
                .collect();
            answer.insert(String::from("maps"), Value::Array(values));
        }
        if let Some(calls) = &input.requests {
            let values = calls.iter().map(reports::request_report).collect();
            answer.insert(String::from("requests"), Value::Array(values));
        }
        if let Some(calls) = &input.responses {
            let values = calls.iter().map(reports::response_report).collect();
            answer.insert(String::from("responses"), Value::Array(values));
        }
        if let Some(calls) = &input.providers {
            let values = calls
                .iter()
                .map(|call| reports::provider_report(&installed, call))
                .collect();
            answer.insert(String::from("providers"), Value::Array(values));
        }
        if let Some(calls) = &input.auths {
            let values = calls
                .iter()
                .map(reports::auth_report)
                .collect::<Result<Vec<_>, _>>()?;
            answer.insert(String::from("auths"), Value::Array(values));
        }
        if input.fields == Some(true) {
            answer.insert(String::from("fields"), json!(required_params()));
        }
        Ok(Value::Object(answer))
    }
}
