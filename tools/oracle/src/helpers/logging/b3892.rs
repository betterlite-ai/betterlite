use betterlite_logging::otel::model::trace_controls::caller_trace_controls;
use betterlite_logging::otel::model::utils::coercion::{OtelModelValue, as_int, as_str_tuple};
use betterlite_logging::otel::model::utils::time::{FixedLocalTimeResolver, to_ns, to_seconds};
use chrono::FixedOffset;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr as _;

pub(crate) struct Op;

#[path = "b3892/tests.rs"]
mod tests;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("trace") => trace(input),
            Some("coercion") => coercions(input),
            Some("time") => time_values(input),
            _ => Err(invalid("unknown b3892 function")),
        }
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

fn trace(input: &Value) -> crate::helpers::HelperResult {
    let parsed = serde_json::from_value::<OtelModelValue>(input.clone())
        .map_err(|error| invalid(&error.to_string()))?;
    serde_json::to_value(caller_trace_controls(&parsed))
        .map_err(|error| invalid(&error.to_string()))
}

fn coercions(input: &Value) -> crate::helpers::HelperResult {
    let values = input
        .get("values")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("values must be an array"))?;
    Ok(Value::Array(values.iter().map(coerce).collect()))
}

fn time_values(input: &Value) -> crate::helpers::HelperResult {
    let values = input
        .get("values")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("values must be an array"))?;
    let offset = FixedOffset::east_opt(0).ok_or_else(|| invalid("UTC offset is invalid"))?;
    let resolver = FixedLocalTimeResolver { offset };
    Ok(Value::Array(
        values
            .iter()
            .map(|value| time_value(value, &resolver))
            .collect(),
    ))
}

fn time_value(value: &Value, resolver: &FixedLocalTimeResolver) -> Value {
    let (seconds, nanoseconds) = match value {
        Value::Number(input_number) => {
            let decimal = Decimal::from_str(&input_number.to_string()).ok();
            let decimal_seconds =
                decimal.and_then(|epoch| to_seconds(Some(&epoch), None, None, resolver));
            let decimal_nanoseconds = decimal.and_then(|epoch| to_ns(Some(&epoch), None, resolver));
            (decimal_seconds, decimal_nanoseconds)
        }
        Value::String(text) => (to_seconds(None, None, Some(text), resolver), None),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => (None, None),
    };
    let seconds_json = seconds.and_then(seconds_number).map(Value::Number);
    json!({"seconds": seconds_json, "nanoseconds": nanoseconds})
}

fn seconds_number(seconds: Decimal) -> Option<serde_json::Number> {
    let mut text = seconds.normalize().to_string();
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    serde_json::Number::from_str(&text).ok()
}

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
        "boolean" => json!(parsed.as_bool()),
        "mapping" => json!(parsed.as_object()),
        "tuple" => json!(as_str_tuple(&parsed)),
        _ => Value::Null,
    }
}
