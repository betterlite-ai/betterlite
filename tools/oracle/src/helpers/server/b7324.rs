use std::time::Duration;

use betterlite_server::keepalive_interval::{
    PingSetting, agreed_keepalive_interval, coerce_keepalive_interval,
};
use betterlite_server::sse_error_frames::{SseFailure, exception_detail_parts, sse_error_payload};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgreedGroup {
    deployments: Vec<Value>,
    global_interval: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Coerce { settings: Vec<Option<Value>> },
    Agreed { groups: Vec<AgreedGroup> },
    DetailParts { details: Vec<Value> },
    ErrorPayload { failures: Vec<SseFailure> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Coerce { settings } => coerce_settings(settings),
            Input::Agreed { groups } => agreed_intervals(groups),
            Input::DetailParts { details } => Ok(detail_parts(details)),
            Input::ErrorPayload { failures } => Ok(error_payloads(failures)),
        }
    }
}

fn coerce_settings(settings: &[Option<Value>]) -> HelperResult {
    let mut results = Vec::with_capacity(settings.len());
    for configured in settings {
        let setting = ping_setting(configured.as_ref())?;
        results.push(seconds_value(coerce_keepalive_interval(setting.as_ref())));
    }
    Ok(Value::Array(results))
}

fn agreed_intervals(groups: &[AgreedGroup]) -> HelperResult {
    let mut results = Vec::with_capacity(groups.len());
    for group in groups {
        let global = ping_setting(group.global_interval.as_ref())?;
        results.push(seconds_value(agreed_keepalive_interval(
            &group.deployments,
            global.as_ref(),
        )));
    }
    Ok(Value::Array(results))
}

fn detail_parts(details: &[Value]) -> Value {
    let mut results = Vec::with_capacity(details.len());
    for detail in details {
        let (message, structured) = exception_detail_parts(detail);
        let mut output = Map::new();
        output.insert(String::from("message"), Value::String(message));
        output.insert(
            String::from("fields"),
            structured.map_or(Value::Null, |object| {
                Value::Object(object.into_iter().collect())
            }),
        );
        results.push(Value::Object(output));
    }
    Value::Array(results)
}

fn error_payloads(failures: &[SseFailure]) -> Value {
    let mut results = Vec::with_capacity(failures.len());
    for failure in failures {
        let (status, body) = sse_error_payload(failure);
        let mut output = Map::new();
        output.insert(String::from("status"), Value::from(status));
        output.insert(
            String::from("body"),
            Value::Object(body.into_iter().collect()),
        );
        results.push(Value::Object(output));
    }
    Value::Array(results)
}

fn ping_setting(value: Option<&Value>) -> Result<Option<PingSetting>, Box<HelperError>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => Ok(number.as_f64().map(PingSetting::Seconds)),
        Some(Value::String(text)) => Ok(Some(PingSetting::Text(text.clone()))),
        Some(other) => Err(Box::new(HelperError {
            class: String::from("TypeError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("the keepalive setting is not a number or string: {other}"),
        })),
    }
}

fn seconds_value(duration: Option<Duration>) -> Value {
    let Some(elapsed) = duration else {
        return Value::Null;
    };
    serde_json::Number::from_f64(elapsed.as_secs_f64()).map_or(Value::Null, Value::Number)
}
