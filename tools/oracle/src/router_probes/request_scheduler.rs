use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;
use async_trait::async_trait;
use betterlite_config::sections::request_scheduler::Settings;
use betterlite_error::error::ErrorClass;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::request_scheduler::error::SchedulerError;
use betterlite_router::request_scheduler::flow::FlowItem;
use betterlite_router::request_scheduler::schedule::ScheduledCall;
use betterlite_router::request_scheduler::service::RequestScheduler;
use betterlite_router::router_core::headers;
use betterlite_router::routing_timeouts::global::GlobalRequestTimeout;
use betterlite_router::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, TimeoutPlan, resolve_router_timeouts,
};
use betterlite_router::routing_timeouts::seconds::Seconds;
use betterlite_router::routing_value_store::{RoutingValueStore, StoreSettings};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::runtime::Builder;
use tokio::time::Instant;
mod wire;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<wire::Payload>);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<wire::OutputFields>);
pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    // a paused clock lets every poll pass at once while the deadline still counts it
    let runtime = Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .map_err(|error| invalid(&error.to_string()))?;
    let fields = runtime.block_on(scenario(&input.0))?;
    Ok(ProbeOutput::RequestScheduler(Output(Box::new(fields))))
}
async fn scenario(payload: &wire::Payload) -> Result<wire::OutputFields, OperationError> {
    let service = RequestScheduler::new(
        local_store()?,
        None,
        settings(payload)?,
        Arc::new(|| "probe-generated".to_owned()),
    );
    if let Some(fill) = &payload.fill {
        for index in 0..fill.count {
            let request_id = format!("{}-{index}", fill.prefix);
            service
                .add_request(&item(fill.priority, &request_id, wire::MODEL))
                .await
                .map_err(|error| invalid(&error.to_string()))?;
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut steps = Vec::with_capacity(payload.steps.len());
    for step in &payload.steps {
        let (outcome, error) = apply(&service, step, &payload.timeout, &calls).await?;
        let queue = service
            .queue(step.model())
            .await
            .map(|queued| queued.value)
            .unwrap_or_default();
        let depth = queue.len();
        steps.push(wire::StepOutput {
            outcome,
            queue,
            depth,
            error,
        });
    }
    Ok(wire::OutputFields {
        steps,
        calls: calls.load(Ordering::SeqCst),
    })
}
type Reply = (wire::Outcome, Option<wire::Failure>);
async fn apply(
    service: &RequestScheduler,
    step: &wire::Step,
    timeout: &str,
    calls: &Arc<AtomicUsize>,
) -> Result<Reply, OperationError> {
    match step {
        wire::Step::Schedule {
            priority,
            request_id,
            healthy,
            ..
        } => {
            let queued = Queued {
                priority: *priority,
                request_id,
                healthy: *healthy,
            };
            let deadline = plan(timeout)?;
            Ok(run_scheduled(service, step.model(), &queued, &deadline, calls).await)
        }
        wire::Step::Queue { .. } => Ok((wire::Outcome::Queue, None)),
        wire::Step::Interval { .. } => Ok((
            wire::Outcome::Interval {
                interval: format!("{:.6}", service.polling_interval().as_secs_f64()),
            },
            None,
        )),
        wire::Step::Add {
            priority,
            request_id,
            ..
        } => Ok(add(service, step.model(), *priority, request_id).await),
        wire::Step::Poll {
            request_id,
            healthy,
            ..
        } => Ok(poll(service, step.model(), request_id, *healthy).await),
        wire::Step::Peek { request_id, .. } => Ok(peek(service, step.model(), request_id).await),
    }
}
async fn add(service: &RequestScheduler, model: &str, priority: u8, request_id: &str) -> Reply {
    let added = service
        .add_request(&item(priority, request_id, model))
        .await;
    answer(added.map(|_added| wire::Outcome::Added))
}
async fn poll(service: &RequestScheduler, model: &str, request_id: &str, healthy: usize) -> Reply {
    let polled = service.poll(request_id, model, healthy).await;
    answer(polled.map(|observed| wire::Outcome::Polled {
        allowed: observed.value,
    }))
}
async fn peek(service: &RequestScheduler, model: &str, request_id: &str) -> Reply {
    let peeked = service.peek(request_id, model).await;
    answer(peeked.map(|observed| wire::Outcome::Peeked {
        head: observed.value,
    }))
}
struct Queued<'queued> {
    priority: u8,
    request_id: &'queued str,
    healthy: usize,
}
// the scripted provider is reached only after the queue lets the request run
async fn run_scheduled(
    service: &RequestScheduler,
    model: &str,
    queued: &Queued<'_>,
    deadline: &TimeoutPlan,
    calls: &Arc<AtomicUsize>,
) -> Reply {
    let call = Scripted {
        healthy: queued.healthy,
        calls: Arc::clone(calls),
    };
    let answered = service
        .schedule(
            &item(queued.priority, queued.request_id, model),
            &call,
            deadline,
        )
        .await;
    answer(answered.map(|outcome| wire::Outcome::Scheduled {
        prioritized: outcome.prioritized,
        priority: outcome.priority,
    }))
}
fn answer(result: Result<wire::Outcome, SchedulerError>) -> Reply {
    match result {
        Ok(outcome) => (outcome, None),
        Err(error) => (
            wire::Outcome::Error,
            Some(wire::Failure {
                status: error.status_code(),
                code: error.code(),
                priority: error.priority(),
            }),
        ),
    }
}
fn item(priority: u8, request_id: &str, model: &str) -> FlowItem {
    FlowItem {
        priority,
        request_id: request_id.to_owned(),
        model: model.to_owned(),
    }
}
fn settings(payload: &wire::Payload) -> Result<Settings, OperationError> {
    let parsed: Option<f64> = payload
        .polling_interval
        .as_deref()
        .map(str::parse::<f64>)
        .transpose()
        .map_err(|_source| invalid("polling_interval is not a number"))?;
    let defaults = Settings::default();
    Ok(Settings {
        polling_interval: parsed.unwrap_or(defaults.polling_interval),
        default_priority: payload.default_priority,
        ..defaults
    })
}
fn local_store() -> Result<RoutingValueStore, OperationError> {
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: Some(1024),
            loaded_max_size_per_item_kib: 1024,
        },
        Arc::new(|| 0.0_f64),
    )
    .map_err(|error| invalid(&error.to_string()))
}
// Router(timeout=...) with no request or deployment timeout, started on the paused clock
fn plan(timeout: &str) -> Result<TimeoutPlan, OperationError> {
    let seconds: f64 = timeout
        .parse()
        .map_err(|_source| invalid(&format!("timeout is not a number: {timeout}")))?;
    let sources = RouterTimeoutSources {
        timeout: Some(
            Seconds::new("router_settings.timeout", seconds)
                .map_err(|error| invalid(&error.to_string()))?,
        ),
        stream_timeout: None,
        default_stream_timeout: None,
    };
    let global = GlobalRequestTimeout {
        seconds: Seconds(6000.0_f64),
        explicitly_set: false,
    };
    let request = RequestTimeouts {
        stream: false,
        timeout: None,
        request_timeout: None,
        stream_timeout: None,
    };
    TimeoutPlan::new(
        Instant::now().into_std(),
        &resolve_router_timeouts(&sources, global),
        &request,
        &DeploymentTimeouts::default(),
    )
    .map_err(|error| invalid(&error.to_string()))
}
struct Scripted {
    healthy: usize,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl ScheduledCall for Scripted {
    fn healthy_deployments(&self) -> usize {
        self.healthy
    }
    async fn call(&self) -> Result<headers::Response, Box<ErrorClass>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(headers::Response::Object(JsonObject::new()))
    }
}
fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}
