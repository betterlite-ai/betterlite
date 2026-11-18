mod logging;
mod tests;
mod warnings;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::response_chat_result::records::ModelResponse;
use betterlite_core::api::response_chat_result::transform::{ResponseSource, transform_response};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use logging::logging_state;
use warnings::Warnings;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    response: Value,
    #[serde(default)]
    caller_extra: Map<String, Value>,
    #[serde(default)]
    caller_hidden_params: Map<String, Value>,
    #[serde(default)]
    raw_hidden_params: Map<String, Value>,
    #[serde(default)]
    recorded: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let value = with_default(subscriber, || answer(input))?;
        Ok(json!({"value": value, "warnings": warnings.labels()}))
    }
}

fn answer(input: &Input) -> HelperResult {
    let mut response = decode(&input.response)?;
    let caller = ModelResponse {
        id: String::new(),
        created: 5,
        model: None,
        object: "chat.completion".to_owned(),
        system_fingerprint: Some("fp".to_owned()),
        choices: Vec::new(),
        usage: None,
        extra: input.caller_extra.clone(),
        hidden_params: input.caller_hidden_params.clone(),
    };
    let source = ResponseSource {
        model: &input.model,
        response: &mut response,
        hidden_params: &input.raw_hidden_params,
    };
    let record = transform_response(
        source,
        caller,
        &logging_state(input.recorded.as_deref())?,
        &ApiLimits::default(),
    )
    .or_value_error()?;
    Ok(json!({
        "record": serde_json::to_value(&record)?,
        "hidden_params": serde_json::to_value(&record.hidden_params)?,
    }))
}

// the pipeline rejects a body no Responses record can hold before this conversion sees it
fn decode(value: &Value) -> Result<ResponsesApiResponse, Box<HelperError>> {
    let Value::Object(_) = value else {
        return Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: format!("Unexpected response type: <class '{}'>", json_class(value)),
        }));
    };
    Ok(serde_json::from_value(value.clone())?)
}

fn json_class(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
