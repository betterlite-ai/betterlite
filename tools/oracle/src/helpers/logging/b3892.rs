use betterlite_logging::otel::model::trace_controls::caller_trace_controls;
use betterlite_logging::otel::model::utils::coercion::{OtelModelValue, as_int, as_str_tuple};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr;

pub(crate) struct Op;

#[path = "b3892/tests.rs"]
mod tests;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input.get("function").and_then(Value::as_str) {
            Some("trace") => {
                let parsed = serde_json::from_value::<OtelModelValue>(input.clone())
                    .map_err(|error| invalid(&error.to_string()))?;
                serde_json::to_value(caller_trace_controls(&parsed))
                    .map_err(|error| invalid(&error.to_string()))?
            }
            Some("coercion") => {
                let values = input
                    .get("values")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("values must be an array"))?;
                Value::Array(values.iter().map(coerce).collect())
            }
            Some("time") => {
                let values = input
                    .get("values")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("values must be an array"))?;
                Value::Array(
                    values
                        .iter()
                        .map(|value| {
                            let decimal = value
                                .as_number()
                                .and_then(|number| Decimal::from_str(&number.to_string()).ok());
                            let nanoseconds = decimal
                                .and_then(|number| {
                                    number.checked_mul(Decimal::from(1_000_000_000_i64))
                                })
                                .and_then(|number| number.trunc().to_i128());
                            let seconds = decimal
                                .and_then(|number| {
                                    serde_json::Number::from_str(&number.to_string()).ok()
                                })
                                .map(Value::Number);
                            json!({"seconds": seconds, "nanoseconds": nanoseconds})
                        })
                        .collect(),
                )
            }
            _ => return Err(invalid("unknown b3892 function")),
        };
        Ok(output)
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

use rust_decimal::prelude::ToPrimitive;

fn coerce(item: &Value) -> Value {
    let Some(kind) = item.get("kind").and_then(Value::as_str) else {
        return Value::Null;
    };
    let Some(value) = item.get("value") else {
        return Value::Null;
    };
    let Ok(parsed) = serde_json::from_value::<OtelModelValue>(value.clone()) else {
        return Value::Null;
    };
    match kind {
        "string" => json!(parsed.as_str()),
        "integer" => json!(as_int(&parsed)),
        "decimal" => json!(
            parsed
                .as_decimal()
                .map(|number| number.normalize().to_string())
        ),
        "boolean" => match parsed {
            OtelModelValue::Bool(value) => json!(value),
            _ => Value::Null,
        },
        "mapping" => match parsed {
            OtelModelValue::Object(map) => json!(map),
            _ => Value::Null,
        },
        "tuple" => json!(as_str_tuple(&parsed)),
        _ => Value::Null,
    }
}
