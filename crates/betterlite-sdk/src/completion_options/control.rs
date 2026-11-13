use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_models::call_helpers::drop_params::normalize_drop_params;
use betterlite_models::controls::control_options::{ControlOptions, parse_control_options};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

use super::error::CompletionOptionsError;
use crate::call_setup::RequestMap;

const STREAM_CHUNK_SIZE_KEY: &str = "stream_chunk_size";
const DROP_PARAMS_KEY: &str = "drop_params";

pub fn resolve_control_options(
    kwargs: &RequestMap,
    model: &str,
    global_drop_params: Option<bool>,
) -> Result<ControlOptions, CompletionOptionsError> {
    let given = kwargs.get(STREAM_CHUNK_SIZE_KEY).map(control_value);
    let invalid = match parse_control_options(given.as_ref()) {
        Ok(control) => return Ok(control),
        Err(invalid) => invalid,
    };
    if global_drop_params == Some(true)
        || normalize_drop_params(kwargs.get(DROP_PARAMS_KEY)) == Some(true)
    {
        return Ok(ControlOptions::default());
    }
    let context = ErrorContext {
        message: invalid.to_string(),
        model: Some(model.to_owned()),
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: Some(invalid.param().to_owned()),
        body_code: None,
    };
    Err(CompletionOptionsError::InvalidControlOption(Box::new(
        ResponseDetails::from(context),
    )))
}

fn control_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(control_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), control_value(field)))
                .collect(),
        ),
    }
}
