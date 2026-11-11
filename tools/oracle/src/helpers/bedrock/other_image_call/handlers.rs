use std::str;
use std::time::Duration;

use betterlite_bedrock::image_call::error::ImageCallError;
use betterlite_bedrock::image_call::execute::{dispatch_request, transform_response};
use betterlite_bedrock::image_call::plan::{
    ImageCall, LogContext, PreparedRequest, prepare_request, resolve_target,
};
use betterlite_bedrock::image_call::transport::{ClientKind, ImageRequest};
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_logging::logging_state::LoggingState;
use serde_json::{Map, Value, json};

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::fixture::{DiscardingSink, logging_state, prepost_env};
use super::{Failure, Harness};

pub(super) fn prepare_value(probe: &super::Probe) -> Result<Value, Failure> {
    let harness = Harness::new(probe)?;
    let mut state = logging_state()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut call = harness.call();
    let prepared = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let target = runtime.block_on(resolve_target(&mut call, &harness.source))?;
        prepare_request(&mut call, &target, &mut log)?
    };
    prepare_json(&prepared, &state)
}

fn prepare_json(prepared: &PreparedRequest, state: &LoggingState) -> Result<Value, Failure> {
    let details = serde_json::to_value(&state.model_call_details)?;
    let arguments = details.get("additional_args");
    Ok(json!({
        "endpoint_url": prepared.endpoint_url,
        "prepared_url": prepared.signed.url,
        "headers": headers_json(&prepared.signed.headers),
        "body": str::from_utf8(&prepared.signed.body)?,
        "data": serde_json::to_value(&prepared.data)?,
        "pre_call": {
            "input": details.get("input"),
            "api_base": arguments.and_then(|args| args.get("api_base")),
            "complete_input_dict": arguments.and_then(|args| args.get("complete_input_dict")),
            "has_headers": arguments
                .and_then(|args| args.get("headers"))
                .is_some_and(|headers| !headers.is_null()),
        },
    }))
}

struct CallRun {
    state: LoggingState,
    supplied_requests: Vec<ImageRequest>,
    factory_requests: Vec<ImageRequest>,
    factory_calls: Vec<(ClientKind, Option<Duration>)>,
    outcome: Result<ModelResponse, ImageCallError>,
}

impl CallRun {
    fn posted(&self) -> Option<(String, Vec<u8>)> {
        let request = if self.supplied_requests.is_empty() {
            self.factory_requests.first()
        } else {
            self.supplied_requests.first()
        }?;
        Some((request.url.clone(), request.body.clone()))
    }
}

fn call_run(probe: &super::Probe) -> Result<CallRun, Failure> {
    let harness = Harness::new(probe)?;
    let mut state = logging_state()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut call = harness.call();
    let outcome = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        run_stages(
            &harness,
            &mut call,
            &mut log,
            &runtime,
            ModelResponse {
                created: Some(probe.created),
                data: Vec::new(),
            },
        )
    };
    let (supplied_requests, factory_requests) = harness.seen();
    Ok(CallRun {
        state,
        supplied_requests,
        factory_requests,
        factory_calls: harness.factory.calls(),
        outcome,
    })
}

fn run_stages(
    harness: &Harness<'_>,
    call: &mut ImageCall<'_>,
    log: &mut LogContext<'_, '_>,
    runtime: &tokio::runtime::Runtime,
    model_response: ModelResponse,
) -> Result<ModelResponse, ImageCallError> {
    let target = runtime.block_on(resolve_target(call, &harness.source))?;
    let prepared = prepare_request(call, &target, log)?;
    let reply = runtime.block_on(dispatch_request(
        call,
        &prepared,
        harness.supplied(),
        &harness.factory,
    ))?;
    transform_response(call, &prepared, &reply, model_response, log)
}

pub(super) fn call_value(probe: &super::Probe) -> Result<Value, Failure> {
    let run = call_run(probe)?;
    let (url, body) = run.posted().ok_or(Failure::Shape)?;
    let response = run.outcome?;
    Ok(json!({
        "created": response.created,
        "data": images_json(&response),
        "posted_url": url,
        "posted_body": serde_json::from_str::<Value>(str::from_utf8(&body)?)?,
        "post_call": post_call_json(&run.state),
    }))
}

pub(super) fn failure_value(probe: &super::Probe) -> Result<Value, Failure> {
    let run = call_run(probe)?;
    let response = run.outcome?;
    Ok(json!({
        "created": response.created,
        "data": images_json(&response),
        "post_call": post_call_json(&run.state),
    }))
}

pub(super) fn error_value(probe: &super::Probe) -> Result<Value, Failure> {
    let run = call_run(probe)?;
    let (ok, status) = match &run.outcome {
        Ok(_) => (true, None),
        Err(error) => (false, error.status()),
    };
    Ok(json!({
        "ok": ok,
        "status": status,
        "post_call": post_call_json(&run.state).is_some(),
    }))
}

pub(super) fn selection_value(probe: &super::Probe) -> Result<Value, Failure> {
    let run = call_run(probe)?;
    let (url, _body) = run.posted().ok_or(Failure::Shape)?;
    run.outcome?;
    let last = run.factory_calls.last();
    let factory = match last {
        None => "none",
        Some((ClientKind::Sync, _)) => "sync",
        Some((ClientKind::Async, _)) => "async",
    };
    let timeout_ms = last
        .and_then(|(_, timeout)| *timeout)
        .and_then(|value| u64::try_from(value.as_millis()).ok());
    Ok(json!({
        "factory": factory,
        "timeout_ms": timeout_ms,
        "supplied_requests": run.supplied_requests.len(),
        "factory_requests": run.factory_requests.len(),
        "posted_url": url,
    }))
}

pub(super) fn runtime_value(probe: &super::Probe) -> Result<Value, Failure> {
    let prepared = prepare_value(probe)?;
    let run = call_run(probe)?;
    let (url, _body) = run.posted().ok_or(Failure::Shape)?;
    run.outcome?;
    let Value::Object(mut fields) = prepared else {
        return Err(Failure::Shape);
    };
    fields.remove("headers");
    fields.insert("posted_url".to_owned(), Value::String(url));
    Ok(Value::Object(fields))
}

fn images_json(response: &ModelResponse) -> Value {
    Value::Array(
        response
            .data
            .iter()
            .map(|image| json!({"b64_json": image.b64_json}))
            .collect(),
    )
}

fn headers_json(headers: &[(String, String)]) -> Value {
    let mut map = Map::new();
    for (name, value) in headers {
        map.insert(name.clone(), Value::String(value.clone()));
    }
    Value::Object(map)
}

fn post_call_json(state: &LoggingState) -> Option<Value> {
    let details = serde_json::to_value(&state.model_call_details).ok()?;
    let original = details.get("original_response")?;
    let arguments = details.get("additional_args")?;
    Some(json!({
        "input": details.get("input"),
        "original_response": original,
        "complete_input_dict": arguments.get("complete_input_dict"),
    }))
}
