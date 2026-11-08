use std::str::FromStr as _;

use betterlite_streaming::stream_chunk_shaping::error::StreamShapeError;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Number, Value};

use crate::helpers::HelperError;

pub(super) fn encoded<T>(value: &T) -> Result<Value, Box<HelperError>>
where
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(serialized) => Ok(serialized),
        Err(error) => Err(failure("SerializationError", None, error.to_string())),
    }
}

pub(super) fn shape_error(error: &StreamShapeError) -> Box<HelperError> {
    let (class, status) = match error {
        StreamShapeError::Repetition { .. } => ("InternalServerError", Some(500_u16)),
        StreamShapeError::Arguments { .. } => ("ValidationError", None),
    };
    failure(class, status, error.to_string())
}

pub(super) fn decimal_json(cost: Decimal) -> Option<Value> {
    Number::from_str(&cost.normalize().to_string())
        .ok()
        .map(Value::Number)
}

pub(super) fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
