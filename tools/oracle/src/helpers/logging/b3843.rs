use betterlite_logging::otel::model::payloads::guardrail::GuardrailSpanData;
use betterlite_logging::otel::model::payloads::{helpers, sanitize_event_metadata};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let function = input
            .get("function")
            .and_then(Value::as_str)
            .unwrap_or_default();
        dispatch(function, input)
    }
}

fn dispatch(function: &str, input: &Value) -> crate::helpers::HelperResult {
    match function {
        "sanitize_event_metadata" => run_sanitize(input),
        "guardrail_error_status" => run_guardrail(input),
        "cache_token_value"
        | "guardrail_mode_str"
        | "total_masked_entities"
        | "dicts"
        | "json_or_none" => run_value_helper(function, input),
        _ => Err(invalid(&format!("unknown B3843 function {function}"))),
    }
}

fn run_value_helper(function: &str, input: &Value) -> crate::helpers::HelperResult {
    let raw_value = input.get("value").unwrap_or(&Value::Null);
    let parsed_value = parse_model_value(raw_value)?;
    let result = match function {
        "cache_token_value" => {
            let values = input
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(parse_model_value)
                .collect::<Result<Vec<_>, _>>()?;
            helpers::cache_token_value(&values).map_or(Value::Null, Value::from)
        }
        "guardrail_mode_str" => {
            helpers::guardrail_mode_str(&parsed_value).map_or(Value::Null, Value::String)
        }
        "total_masked_entities" => {
            helpers::total_masked_entities(&parsed_value).map_or(Value::Null, Value::from)
        }
        "dicts" => serde_json::json!(helpers::dicts(&parsed_value)),
        "json_or_none" => helpers::json_or_none(&parsed_value).map_or(Value::Null, Value::String),
        _ => return Err(invalid("unsupported B3843 value helper")),
    };
    Ok(result)
}

fn run_sanitize(input: &Value) -> crate::helpers::HelperResult {
    let fields = parse_fields(input, "value", "event metadata must be an object")?;
    Ok(serde_json::json!(
        sanitize_event_metadata::sanitize_event_metadata(&fields)
    ))
}

fn run_guardrail(input: &Value) -> crate::helpers::HelperResult {
    let fields = parse_fields(input, "entry", "guardrail entry must be an object")?;
    let parsed = GuardrailSpanData::from_logging_entry(&fields);
    Ok(
        serde_json::json!({"status": parsed.status, "error_type": parsed.error.and_then(|error| error.error_type)}),
    )
}

fn parse_fields(
    input: &Value,
    field: &str,
    error_message: &str,
) -> Result<BTreeMap<String, OtelModelValue>, Box<crate::helpers::HelperError>> {
    let raw_fields = input
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(error_message))?;
    raw_fields
        .iter()
        .map(|(key, raw)| parse_model_value(raw).map(|parsed| (key.clone(), parsed)))
        .collect()
}

fn parse_model_value(value: &Value) -> Result<OtelModelValue, Box<crate::helpers::HelperError>> {
    match serde_json::from_value(value.clone()) {
        Ok(parsed) => Ok(parsed),
        Err(error) => Err(invalid(&error.to_string())),
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

mod tests;
