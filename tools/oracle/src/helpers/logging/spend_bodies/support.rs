use betterlite_models::decisions::Json;
use serde_json::Value;

use crate::helpers::HelperError;

pub(super) fn decode(value: &Value) -> Result<Json, Box<HelperError>> {
    let decoded =
        serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))?;
    Ok(decoded)
}

pub(super) fn decode_optional(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    match value {
        Some(present) => Ok(Some(decode(present)?)),
        None => Ok(None),
    }
}

pub(super) fn required<T>(value: Option<T>, name: &str) -> Result<T, Box<HelperError>> {
    let Some(present) = value else {
        return Err(invalid(name));
    };
    Ok(present)
}

pub(super) fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
