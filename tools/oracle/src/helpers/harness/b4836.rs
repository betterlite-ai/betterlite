mod events;

use betterlite_harness::providers::opencode::transformation::OpenCodeHarnessConfig;
use betterlite_harness::transformation::BaseCLIHarnessConfig as _;
use betterlite_harness::types::JsonObject;
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    stream: Option<String>,
    requests: Option<String>,
}

fn stream(input: &str) -> HelperResult {
    let config = OpenCodeHarnessConfig;
    let mut state = config.create_stream_state();
    let mut started = BTreeSet::new();
    let mut output = Vec::new();
    for raw_line in input.lines().filter(|line| !line.is_empty()) {
        let line: JsonObject = serde_json::from_str(raw_line).map_err(|error| {
            Box::new(HelperError {
                class: "StreamParse".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        for event in config.transform_stream_line(&line, &mut state) {
            output.push(events::encode(&event, &mut started)?);
        }
    }
    Ok(json!({
        "session_id": state.session_id,
        "final_text": state.final_text,
        "error": state.error,
        "events": output,
    }))
}

fn requests(input: &str) -> HelperResult {
    let requests = serde_json::Deserializer::from_str(input)
        .into_iter::<Value>()
        .map(|request| request.map(|value| project_request(&value)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            Box::new(HelperError {
                class: "RequestParse".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    Ok(json!({"requests": requests}))
}

fn project_request(request: &Value) -> Value {
    let object = request.as_object().cloned().unwrap_or_default();
    let headers = object
        .get("headers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    json!({
        "auth_prefix": object.get("auth_prefix"), "content_type": headers.get("Content-Type"),
        "tools": object.get("tools").cloned().unwrap_or_else(|| json!([])),
        "n_messages": object.get("n_messages"), "roles": object.get("roles").cloned().unwrap_or_else(|| json!([])),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let result = match (&input.stream, &input.requests) {
            (Some(stream_input), None) => stream(stream_input),
            (None, Some(request_input)) => requests(request_input),
            _ => {
                return Err(Box::new(HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: "b4836 input must contain exactly one stream or requests string"
                        .to_owned(),
                }));
            }
        }?;
        Ok(json!({"result": result}))
    }
}
