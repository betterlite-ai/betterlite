use betterlite_logging::fields::messages::append_system_prompt_messages;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::payload::records::{AdditionalHeaders, GuardrailInformation};
use betterlite_logging::fields::response::{HiddenParams, InitResponseObject, extract};
use betterlite_logging::fields::status::{LoggingStatus, get_status_fields};
use betterlite_logging::fields::timestamps::{Moment, cleanup_timestamps};
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::fields::usage::{ResponseUsage, Usage, get_usage_as_dict};
use betterlite_models::decisions::Json;
use chrono::DateTime;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::str::FromStr as _;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("cleanup_timestamps") => cleanup(input),
            Some("system_messages") => messages(input),
            Some("status_fields") => status(input),
            Some("usage_as_dict") => usage(input),
            Some("extract_response") => extracted(input),
            Some("payload_record") => round_trip::<StandardLoggingPayload>(input, "payload"),
            _ => Err(invalid("unknown logging_fields function")),
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

fn encode<T>(value: T) -> crate::helpers::HelperResult
where
    T: Serialize,
{
    match serde_json::to_value(value) {
        Ok(encoded) => Ok(encoded),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

fn optional_value<T>(
    input: &Value,
    name: &str,
) -> Result<Option<T>, Box<crate::helpers::HelperError>>
where
    T: DeserializeOwned,
{
    let Some(value) = input.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let parsed =
        serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))?;
    Ok(Some(parsed))
}

fn decode_moment(
    encoded: Option<&Value>,
) -> Result<Option<Moment>, Box<crate::helpers::HelperError>> {
    match encoded {
        Some(Value::String(text)) => {
            let parsed =
                DateTime::parse_from_rfc3339(text).map_err(|error| invalid(&error.to_string()))?;
            Ok(Some(Moment::DateTime(parsed)))
        }
        Some(Value::Number(number)) => {
            let text = number.to_string();
            if text.contains(['.', 'e', 'E']) {
                let parsed = if text.contains(['e', 'E']) {
                    Decimal::from_scientific(&text)
                } else {
                    Decimal::from_str(&text)
                };
                let value = parsed.map_err(|error| invalid(&error.to_string()))?;
                Ok(Some(Moment::Float(value)))
            } else {
                let value = text
                    .parse::<i64>()
                    .map_err(|error| invalid(&error.to_string()))?;
                Ok(Some(Moment::Integer(value)))
            }
        }
        Some(Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_)) | None => Ok(None),
    }
}

fn moments(input: &Value) -> Result<[Option<Moment>; 3], Box<crate::helpers::HelperError>> {
    let start = decode_moment(input.get("start"))?;
    let end = decode_moment(input.get("end"))?;
    let completion = decode_moment(input.get("completion_start"))?;
    Ok([start, end, completion])
}

fn cleanup(input: &Value) -> crate::helpers::HelperResult {
    let [start, end, completion] = moments(input)?;
    match cleanup_timestamps(start.as_ref(), end.as_ref(), completion.as_ref()) {
        Ok(values) => encode(values),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

fn round_trip<T>(input: &Value, name: &str) -> crate::helpers::HelperResult
where
    T: DeserializeOwned + Serialize,
{
    if let Some(parsed) = optional_value::<T>(input, name)? {
        return encode(parsed);
    }
    Err(invalid("a required value is missing"))
}

fn messages(input: &Value) -> crate::helpers::HelperResult {
    let system = input.get("system").and_then(Value::as_str);
    let decoded = optional_value::<Json>(input, "messages")?;
    encode(append_system_prompt_messages(system, decoded.as_ref()))
}

fn status(input: &Value) -> crate::helpers::HelperResult {
    let parsed = optional_value::<LoggingStatus>(input, "status")?;
    let status = parsed.ok_or_else(|| invalid("status is required"))?;
    let guardrails = optional_value::<Vec<GuardrailInformation>>(input, "guardrail_information")?;
    encode(get_status_fields(status, guardrails.as_deref()))
}

fn combined_usage(input: &Value) -> Result<Option<Usage>, Box<crate::helpers::HelperError>> {
    let mapping = optional_value::<IndexMap<String, Json>>(input, "combined")?;
    mapping
        .map(|combined| usage_from_mapping(&combined).map_err(|error| invalid(&error.to_string())))
        .transpose()
}

fn usage(input: &Value) -> crate::helpers::HelperResult {
    let combined = combined_usage(input)?;
    let response = optional_value::<IndexMap<String, Json>>(input, "usage")?
        .map_or(ResponseUsage::Absent, ResponseUsage::Mapping);
    encode(get_usage_as_dict(response, combined.as_ref()))
}

fn decode_init(
    encoded: Option<&Value>,
) -> Result<InitResponseObject, Box<crate::helpers::HelperError>> {
    let Some(value) = encoded else {
        return Ok(InitResponseObject::Absent);
    };
    let Value::Object(fields) = value else {
        return Ok(InitResponseObject::Other);
    };
    let Some(body) = fields.get("mapping") else {
        return Ok(InitResponseObject::Other);
    };
    let mapping =
        serde_json::from_value(body.clone()).map_err(|error| invalid(&error.to_string()))?;
    Ok(InitResponseObject::Mapping(mapping))
}

fn extracted(input: &Value) -> crate::helpers::HelperResult {
    let headers = optional_value::<AdditionalHeaders>(input, "additional_headers")?;
    let extracted = extract(decode_init(input.get("init"))?, headers);
    let hidden = match extracted.hidden_params {
        Some(HiddenParams::FromObject(fields)) => encode(Json::Object(fields))?,
        Some(HiddenParams::FromException(record)) => encode(record)?,
        None => Value::Null,
    };
    Ok(json!({
        "response_obj": Json::Object(extracted.response_obj),
        "hidden_params": hidden,
    }))
}
