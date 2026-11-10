mod input;
mod support;
mod tests;

use betterlite_logging::otel::legacy_attributes::dispatch::set_attributes;
use betterlite_logging::otel::legacy_attributes::raw_request::set_raw_request_attributes;
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use serde_json::Value;

use crate::helpers::HelperResult;

use input::{CallbackNames, Input};
use support::{capture_mode, config, context, invalid, record};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "set_attributes" => attributes_record(input),
            "set_raw_request_attributes" => raw_request_record(input),
            other => Err(invalid(&format!("unknown function {other}"))),
        }
    }
}

fn attributes_record(input: &Input) -> HelperResult {
    let settings = config(input);
    if let Some(CallbackNames::Many(names)) = &input.callback_name {
        return records(input, &settings, names);
    }
    let name = match &input.callback_name {
        Some(CallbackNames::One(name)) => Some(name.as_str()),
        _ => None,
    };
    let record = named_record(input, &settings, name)?;
    Ok(serde_json::json!({"result": record}))
}

fn records(input: &Input, settings: &OpenTelemetryConfig, names: &[String]) -> HelperResult {
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        let mut record = named_record(input, settings, Some(name))?;
        record.insert("callback_name".to_owned(), Value::String(name.clone()));
        entries.push(Value::Object(record));
    }
    Ok(serde_json::json!({"result": {"records": entries}}))
}

fn named_record(
    input: &Input,
    settings: &OpenTelemetryConfig,
    name: Option<&str>,
) -> Result<serde_json::Map<String, Value>, Box<crate::helpers::HelperError>> {
    let context = context(settings, name, capture_mode(input.capture_mode.as_deref()));
    let (attributes, result) =
        record(|span| set_attributes(span, &context, &input.kwargs, input.response.as_ref()))?;
    let mut record = serde_json::Map::new();
    if let Err(error) = result {
        record.insert("error".to_owned(), Value::String(error.to_string()));
    }
    record.insert("attributes".to_owned(), Value::Object(attributes));
    Ok(record)
}

fn raw_request_record(input: &Input) -> HelperResult {
    let (attributes, ()) = record(|span| set_raw_request_attributes(span, &input.kwargs))?;
    Ok(serde_json::json!({"result": {"attributes": attributes}}))
}
