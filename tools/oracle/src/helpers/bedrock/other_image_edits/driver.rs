use std::str;
use std::time::Duration;

use betterlite_bedrock::canvas_results::COST_HEADER;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::image_call::transport::{ClientKind, ImageRequest};
use betterlite_bedrock::image_edits::execute::dispatch_request;
use betterlite_bedrock::image_edits::plan::{EditCall, EditPlan, PreparedRequest};
use betterlite_bedrock::image_edits::prepare::{prepare_request, resolve_target};
use betterlite_bedrock::image_edits::response::{EditReply, ObservedReply, transform_response};
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Runtime};

use super::Failure;
use super::fixtures::{
    Catalog, DiscardingSink, Harness, NOW_SECONDS, Prices, logging_state, prepost_env,
};
use super::input::{Function, Probe};

// The signature carries the clock, so its headers never enter the shared answer; httpx adds the rest.
const IGNORED_HEADERS: [&str; 8] = [
    "accept",
    "accept-encoding",
    "authorization",
    "connection",
    "host",
    "user-agent",
    "x-amz-date",
    "x-amz-security-token",
];

struct Run {
    state: LoggingState,
    prepared: Option<Value>,
    supplied_requests: Vec<ImageRequest>,
    factory_requests: Vec<ImageRequest>,
    factory_calls: Vec<(ClientKind, Option<Duration>)>,
}

type Outcome = Result<EditReply, Failure>;

fn run(probe: &Probe, model_cost: &ParamMap) -> Result<(Run, Outcome), Failure> {
    let harness = Harness::new(probe)?;
    let catalog = Catalog::new()?;
    let context = catalog.context();
    let plan = EditPlan {
        model_cost,
        resolution: &context,
    };
    let mut state = logging_state()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let runtime = Builder::new_current_thread().build()?;
    let mut prepared = None;
    let outcome = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        run_stages(&harness, &runtime, &plan, &mut log, &mut prepared)
    };
    let (supplied_requests, factory_requests) = harness.seen();
    let record = Run {
        state,
        prepared,
        supplied_requests,
        factory_requests,
        factory_calls: harness.factory.calls(),
    };
    Ok((record, outcome))
}

fn run_stages(
    harness: &Harness<'_>,
    runtime: &Runtime,
    plan: &EditPlan<'_, '_>,
    log: &mut LogContext<'_, '_>,
    snapshot: &mut Option<Value>,
) -> Outcome {
    let mut call = harness.call();
    let target = runtime.block_on(resolve_target(&mut call, &harness.source))?;
    let prepared = prepare_request(&mut call, plan, &target, log)?;
    *snapshot = Some(prepare_json(&prepared, &call, log.state)?);
    let reply = runtime.block_on(dispatch_request(
        &call,
        &prepared,
        harness.supplied(),
        &harness.factory,
    ))?;
    let observed = ObservedReply {
        prepared: &prepared,
        reply: &reply,
        now_seconds: NOW_SECONDS,
    };
    Ok(transform_response(
        &call,
        plan,
        &observed,
        &Prices(plan.model_cost),
        log,
    )?)
}

pub(super) fn observe(
    probe: &Probe,
    function: Function,
    model_cost: &ParamMap,
) -> Result<Value, Failure> {
    let (run, outcome) = run(probe, model_cost)?;
    match function {
        Function::Prepare => run
            .prepared
            .ok_or_else(|| outcome.err().unwrap_or(Failure::Shape)),
        Function::Result => Ok(run.result(&outcome)),
        Function::Error => Ok(run.error(&outcome)),
        Function::ModelError => Ok(model_error(&outcome)),
        Function::Selection => {
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

    fn result(&self, outcome: &Outcome) -> Value {
        let post = post_call_json(&self.state);
        match outcome {
            Ok(reply) => json!({
                "ok": true,
                "images": reply.response.data.iter().flatten()
                    .filter_map(|image| image.b64_json.as_deref())
                    .map(|b64| json!({"b64_json": b64}))
                    .collect::<Vec<Value>>(),
                "cost": reply.additional_headers.get(COST_HEADER),
                "post_call": post,
            }),
            Err(error) => json!({
                "ok": false,
                "status": error.status(),
                "post_call": post.is_some(),
            }),
        }
    }

    fn error(&self, outcome: &Outcome) -> Value {
        let status = outcome.as_ref().err().and_then(Failure::status);
        json!({"ok": outcome.is_ok(), "status": status,
               "post_call": post_call_json(&self.state).is_some()})
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
            "timeout_ms": last.and_then(|(_, timeout)| *timeout)
                .and_then(|value| u64::try_from(value.as_millis()).ok()),
            "supplied_requests": self.supplied_requests.len(),
            "factory_requests": self.factory_requests.len(),
            "posted_url": self.posted().map(|request| request.url.as_str()),
        })
    }
}

fn model_error(outcome: &Outcome) -> Value {
    match outcome {
        Ok(_) => json!({"ok": true, "status": null, "message": null}),
        Err(error) => json!({"ok": false, "status": error.status(),
                             "message": error.to_string()}),
    }
}

fn prepare_json(
    prepared: &PreparedRequest,
    call: &EditCall<'_>,
    state: &LoggingState,
) -> Result<Value, Failure> {
    let details = serde_json::to_value(&state.model_call_details)?;
    let arguments = details.get("additional_args");
    let mut remaining: Vec<&str> = call.optional_params.keys().map(String::as_str).collect();
    remaining.sort_unstable();
    let headers: Map<String, Value> = prepared
        .signed
        .headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), Value::String(value.clone())))
        .filter(|(name, _)| !IGNORED_HEADERS.contains(&name.as_str()))
        .collect();
    Ok(json!({
        "endpoint_url": prepared.endpoint_url,
        "headers": headers,
        "body": str::from_utf8(&prepared.signed.body)?,
        "data": serde_json::to_value(&prepared.data)?,
        "remaining_params": remaining,
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
