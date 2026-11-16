use betterlite_foundry::chat::fields::required_params;
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
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
        insert_urls(input, &mut answer);
        insert_supported(input, &runtime, &mut installed, &mut answer);
        insert_maps(input, &runtime, &mut installed, &mut answer);
        insert_requests(input, &mut answer);
        insert_responses(input, &mut answer);
        insert_providers(input, &installed, &mut answer);
        insert_auths(input, &mut answer)?;
        insert_fields(input, &mut answer);
        Ok(Value::Object(answer))
    }
}

fn insert_urls(input: &Input, answer: &mut Map<String, Value>) {
    let Some(calls) = &input.urls else {
        return;
    };
    let values = calls.iter().map(reports::url_report).collect();
    answer.insert(String::from("urls"), Value::Array(values));
}

fn insert_supported(
    input: &Input,
    runtime: &Runtime,
    installed: &mut Installed,
    answer: &mut Map<String, Value>,
) {
    let Some(models) = &input.supported else {
        return;
    };
    let values = models
        .iter()
        .map(|model| reports::supported_report(runtime, installed, model))
        .collect();
    answer.insert(String::from("supported"), Value::Array(values));
}

fn insert_maps(
    input: &Input,
    runtime: &Runtime,
    installed: &mut Installed,
    answer: &mut Map<String, Value>,
) {
    let Some(calls) = &input.maps else {
        return;
    };
    let values = calls
        .iter()
        .map(|call| reports::map_report(runtime, installed, call))
        .collect();
    answer.insert(String::from("maps"), Value::Array(values));
}

fn insert_requests(input: &Input, answer: &mut Map<String, Value>) {
    let Some(calls) = &input.requests else {
        return;
    };
    let values = calls.iter().map(reports::request_report).collect();
    answer.insert(String::from("requests"), Value::Array(values));
}

fn insert_responses(input: &Input, answer: &mut Map<String, Value>) {
    let Some(calls) = &input.responses else {
        return;
    };
    let values = calls.iter().map(reports::response_report).collect();
    answer.insert(String::from("responses"), Value::Array(values));
}

fn insert_providers(input: &Input, installed: &Installed, answer: &mut Map<String, Value>) {
    let Some(calls) = &input.providers else {
        return;
    };
    let values = calls
        .iter()
        .map(|call| reports::provider_report(installed, call))
        .collect();
    answer.insert(String::from("providers"), Value::Array(values));
}

fn insert_auths(input: &Input, answer: &mut Map<String, Value>) -> Result<(), Box<HelperError>> {
    let Some(calls) = &input.auths else {
        return Ok(());
    };
    let values = calls
        .iter()
        .map(reports::auth_report)
        .collect::<Result<Vec<_>, _>>()?;
    answer.insert(String::from("auths"), Value::Array(values));
    Ok(())
}

fn insert_fields(input: &Input, answer: &mut Map<String, Value>) {
    if input.fields == Some(true) {
        answer.insert(String::from("fields"), json!(required_params()));
    }
}
