use std::str;
use std::time::Duration;

use betterlite_bedrock::image_call::execute::{dispatch_request, transform_response};
use betterlite_bedrock::image_call::plan::{
    LogContext, PreparedRequest, prepare_request, resolve_target,
};
use betterlite_bedrock::image_call::transport::{ClientKind, ImageRequest};
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Map, Value, json};

use super::fixtures::{DiscardingSink, logging_state, prepost_env};
use super::{Failure, Harness, Probe};

pub(super) enum Shape {
    Prepare,
    Runtime,
    Dispatch,
    Result,
    Error,
    Selection,
}

struct Run {
    state: LoggingState,
    prepared: Option<Value>,
    supplied_requests: Vec<ImageRequest>,
    factory_requests: Vec<ImageRequest>,
    factory_calls: Vec<(ClientKind, Option<Duration>)>,
}

type Outcome = Result<ModelResponse, Failure>;

fn run(probe: &Probe) -> Result<(Run, Outcome), Failure> {
    let harness = Harness::new(probe)?;
    let mut state = logging_state()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut prepared = None;
    let outcome = run_stages(
        &harness,
        &runtime,
        &mut LogContext {
            state: &mut state,
            env: &env,
        },
        probe.created,
        &mut prepared,
    );
    let (supplied_requests, factory_requests) = harness.seen();
    let run = Run {
        state,
        prepared,
        supplied_requests,
        factory_requests,
        factory_calls: harness.factory.calls(),
    };
    Ok((run, outcome))
}

fn run_stages(
    harness: &Harness<'_>,
    runtime: &tokio::runtime::Runtime,
    log: &mut LogContext<'_, '_>,
    created: i64,
    snapshot: &mut Option<Value>,
) -> Result<ModelResponse, Failure> {
    let mut call = harness.call();
    let target = runtime.block_on(resolve_target(&mut call, &harness.source))?;
    let prepared = prepare_request(&mut call, &target, log)?;
    *snapshot = Some(prepare_json(&prepared, log.state)?);
    let reply = runtime.block_on(dispatch_request(
        &call,
        &prepared,
        harness.supplied(),
        &harness.factory,
    ))?;
    let model_response = ModelResponse {
        created: Some(created),
        data: Vec::new(),
    };
    Ok(transform_response(
        &call,
        &prepared,
        &reply,
        model_response,
        log,
    )?)
}

pub(super) fn observe(probe: &Probe, shape: &Shape) -> Result<Value, Failure> {
    let (run, outcome) = run(probe)?;
    match shape {
        Shape::Error => Ok(json!({
            "ok": outcome.is_ok(),
            "status": outcome.as_ref().err().and_then(Failure::status),
            "post_call": post_call_json(&run.state).is_some(),
        })),
        Shape::Prepare => run
            .prepared
            .ok_or_else(|| outcome.err().unwrap_or(Failure::Shape)),
        Shape::Runtime => {
            outcome?;
            let url = run.posted().map(|sent| sent.url.clone());
            let Some(Value::Object(mut fields)) = run.prepared else {
                return Err(Failure::Shape);
            };
            fields.insert("posted_url".to_owned(), Value::from(url));
            Ok(Value::Object(fields))
        }
        Shape::Dispatch | Shape::Result => run.dispatch(&outcome?),
        Shape::Selection => {
            outcome?;
            Ok(run.selection())
        }
    }
}

impl Run {
    fn posted(&self) -> Option<&ImageRequest> {
        self.supplied_requests
            .first()
            .or_else(|| self.factory_requests.first())
    }

    fn dispatch(&self, response: &ModelResponse) -> Result<Value, Failure> {
        let request = self.posted().ok_or(Failure::Shape)?;
        Ok(json!({
            "created": response.created,
            "data": response
                .data
                .iter()
                .map(|image| json!({"b64_json": image.b64_json}))
                .collect::<Vec<_>>(),
            "posted_url": request.url,
            "posted_body": serde_json::from_slice::<Value>(&request.body)?,
            "post_call": post_call_json(&self.state),
        }))
    }

    fn selection(&self) -> Value {
        let last = self.factory_calls.last();
        let kind = last.map(|(kind, _)| kind);
        json!({
            "factory": match kind {
                None => "none",
                Some(ClientKind::Sync) => "sync",
                Some(ClientKind::Async) => "async",
            },
            "timeout_ms": last
                .and_then(|(_, timeout)| *timeout)
                .and_then(|value| u64::try_from(value.as_millis()).ok()),
            "supplied_requests": self.supplied_requests.len(),
            "factory_requests": self.factory_requests.len(),
            "posted_url": self.posted().map(|request| request.url.as_str()),
        })
    }
}

fn prepare_json(prepared: &PreparedRequest, state: &LoggingState) -> Result<Value, Failure> {
    let details = serde_json::to_value(&state.model_call_details)?;
    let arguments = details.get("additional_args");
    let signed: Map<String, Value> = prepared
        .signed
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    Ok(json!({
        "endpoint_url": prepared.endpoint_url,
        "prepared_url": prepared.signed.url,
        "headers": signed,
        "body": str::from_utf8(&prepared.signed.body)?,
        "data": serde_json::to_value(&prepared.data)?,
        "pre_call": {
            "input": details.get("input"),
            "api_base": arguments.and_then(|args| args.get("api_base")),
            "complete_input_dict": arguments.and_then(|args| args.get("complete_input_dict")),
        },
    }))
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
