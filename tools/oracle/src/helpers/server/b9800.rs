use betterlite_config::sections::settings::JsonValue;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::messages_call::MessagesCallError;
use betterlite_server::messages_call::steps::{
    CallStepError, mock_params_disallowed, required_param_missing,
};
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    gate: Gate,
    #[serde(default)]
    route_type: String,
    #[serde(default)]
    data: JsonValue,
    #[serde(default)]
    allowed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Gate {
    RequiredParams,
    MockParams,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let JsonValue::Object(data) = &input.data else {
            return Err(invalid_data());
        };
        match input.gate {
            Gate::RequiredParams => match required_param_missing(&input.route_type, data) {
                Ok(()) => Ok(JsonValue::Null),
                Err(error) => Ok(missing_param_answer(&error)),
            },
            Gate::MockParams => match mock_params_disallowed(data, input.allowed) {
                Ok(()) => Ok(JsonValue::Null),
                Err(error) => {
                    let failure = MessagesCallError::from(error);
                    Ok(json!({
                        "status": failure.status(),
                        "detail": failure.http_detail(),
                    }))
                }
            },
        }
    }
}

// ProxyMissingRequiredParamError's ProxyException fields, as its handler serializes them
fn missing_param_answer(error: &CallStepError) -> JsonValue {
    json!({
        "message": error.to_string(),
        "type": error.error_type(),
        "param": error.param(),
        "code": error.code(),
    })
}

fn invalid_data() -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: "data must be an object".to_owned(),
    })
}
