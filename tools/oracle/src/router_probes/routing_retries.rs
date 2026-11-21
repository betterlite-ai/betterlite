use std::collections::{BTreeMap, VecDeque};

use betterlite_config::sections::retry_policy::Settings;
use betterlite_error::error::ErrorClass;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::retry_policy::policy::RetryPolicies;
use betterlite_router::router_core::headers::{
    Response, ensure_additional_headers_dict, prepare_response,
};
use betterlite_router::routing_retries::error::RetryError;
use betterlite_router::routing_retries::headers::install_retry_headers;
use betterlite_router::routing_retries::run::{Retried, RetryRun, run as retried};
use betterlite_router::routing_retries::trail::RetryTrail;
use betterlite_router::routing_timeouts::error::TimeoutError;
use betterlite_router::routing_timeouts::global::GlobalRequestTimeout;
use betterlite_router::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, TimeoutPlan, resolve_router_timeouts,
};
use betterlite_router::routing_timeouts::seconds::Seconds;
use betterlite_server::routing::retry_policy::resolve;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tokio::time::Instant;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod dispatch;
mod wire;

const GROUP: &str = "probe-group";
// litellm.request_timeout, which a router given its own timeout never reads
const GLOBAL_TIMEOUT: Seconds = Seconds(6000.0_f64);

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<wire::Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<wire::Fields>);

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    // a paused clock lets every backoff pass at once while the deadline still counts it
    let runtime = Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let fields = runtime.block_on(scenario(&input.0, random));
    // LiteLLM raises when random() has no injected draw left, and the fallback sample must not hide that
    if random.exhausted {
        return Err(OperationError::SamplesExhausted {
            message: "the request has no sample left for this draw".to_owned(),
        });
    }
    let answered = fields.map_err(|message| OperationError::InvalidRequest { message })?;
    Ok(ProbeOutput::RoutingRetries(Output(Box::new(answered))))
}

async fn scenario(
    payload: &wire::Payload,
    random: &mut InjectedRandom<'_>,
) -> Result<wire::Fields, String> {
    let timeout: f64 = payload
        .timeout
        .parse()
        .map_err(|_source| format!("timeout is not a number: {}", payload.timeout))?;
    let sources = RouterTimeoutSources {
        timeout: Some(
            Seconds::new("router_settings.timeout", timeout).map_err(|error| error.to_string())?,
        ),
        stream_timeout: None,
        default_stream_timeout: None,
    };
    let global = GlobalRequestTimeout {
        seconds: GLOBAL_TIMEOUT,
        explicitly_set: false,
    };
    let request = RequestTimeouts {
        stream: payload.stream,
        ..RequestTimeouts::default()
    };
    let plan = TimeoutPlan::new(
        Instant::now().into_std(),
        &resolve_router_timeouts(&sources, global),
        &request,
        &DeploymentTimeouts::default(),
    )
    .map_err(|error| error.to_string())?;
    let policies = RetryPolicies {
        global: payload.retry_policy.clone(),
        model_group: BTreeMap::new(),
    };
    let settings = RetryRun {
        model_group: GROUP,
        model_group_size: payload.deployments.len(),
        request_num_retries: payload.request_num_retries,
        router_num_retries: payload.num_retries,
        policies: &policies,
        // the server resolves its backoff from this section, so the probe reads the same defaults
        backoff: resolve(&Settings::default()).backoff,
        plan: &plan,
    };
    let mut dispatch = dispatch::Scripted {
        deployments: &payload.deployments,
        steps: payload.steps.iter().cloned().collect::<VecDeque<_>>(),
        routing: plan.cancellation.clone(),
        calls: Vec::new(),
        fault: None,
    };
    let Retried { outcome, trail } = retried(&mut dispatch, &settings, random).await;
    if let Some(fault) = dispatch.fault {
        return Err(fault);
    }
    let mut fields = wire::Fields {
        outcome: wire::Outcome::Answered,
        calls: dispatch.calls,
        backoffs: trail
            .backoffs
            .iter()
            .map(|delay| format!("{:.6}", delay.as_secs_f64()))
            .collect(),
        headers: None,
        error: None,
    };
    match outcome {
        Ok(()) => fields.headers = Some(headers(payload.stream, &trail)?),
        Err(RetryError::Provider(error)) => {
            fields.outcome = wire::Outcome::Failed;
            fields.error = Some(failure(&error));
        }
        Err(RetryError::Timeout(TimeoutError::Cancelled)) => {
            fields.outcome = wire::Outcome::Cancelled;
        }
        Err(RetryError::Timeout(TimeoutError::DeadlineExpired | TimeoutError::AttemptExpired)) => {
            fields.outcome = wire::Outcome::TimedOut
        }
        Err(
            error @ (RetryError::Routing(_)
            | RetryError::Backoff { .. }
            | RetryError::Timeout(
                TimeoutError::NotFinite { .. }
                | TimeoutError::Negative { .. }
                | TimeoutError::OutOfRange { .. },
            )),
        ) => return Err(error.to_string()),
    }
    Ok(fields)
}

// add_retry_headers_to_response on the response kind the case asked for, read back from its hidden params
fn headers(stream: bool, trail: &RetryTrail) -> Result<wire::Headers, String> {
    let response = if stream {
        Response::Stream
    } else {
        Response::Object(JsonObject::new())
    };
    let mut prepared = prepare_response(response)
        .ok_or_else(|| "the response takes no hidden params".to_owned())?;
    install_retry_headers(&mut prepared, trail);
    let additional = ensure_additional_headers_dict(&prepared.hidden_params());
    Ok(wire::Headers {
        attempted_retries: header(&additional, "x-litellm-attempted-retries")
            .ok_or_else(|| "the retried response has no attempt count".to_owned())?,
        max_retries: header(&additional, "x-litellm-max-retries"),
    })
}

fn header(headers: &AdditionalHeaders, name: &str) -> Option<i64> {
    match headers.get(name) {
        Some(Json::Number(number)) => number.as_i64(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn failure(error: &ErrorClass) -> wire::Failure {
    let metadata = error.metadata();
    wire::Failure {
        status: metadata.status_code,
        code: metadata.code,
        max_retries: metadata.retry.max_retries,
        num_retries: metadata.retry.num_retries,
    }
}
