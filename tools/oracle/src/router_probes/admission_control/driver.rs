use std::collections::BTreeMap;
use std::str;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::sections::admission_control::Settings;
use betterlite_config::sections::request_root_paths;
use betterlite_server::admission_control::in_flight::InFlightRequests;
use betterlite_server::admission_control::{AdmissionControl, AdmissionStack};
use betterlite_server::middleware::request_root_paths::RootPaths;
use serde_json::{Number, Value, from_str as parse_json};
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::{sleep, timeout};
use tower::ServiceExt as _;

use crate::router_probes::error::OperationError;

use super::app::{self, App};
use super::{Input, LimitsOutput, Observation, RequestSpec, Step, invalid};

const WAIT: Duration = Duration::from_secs(5);
const BODY_LIMIT: usize = 1 << 20;

pub(super) type Answer = (StatusCode, HeaderMap, Bytes);

pub(super) struct Driver {
    router: Router,
    admission: Arc<AdmissionControl>,
    in_flight: Arc<InFlightRequests>,
    specs: BTreeMap<u16, RequestSpec>,
    tasks: JoinSet<(u16, Answer)>,
    aborts: BTreeMap<u16, AbortHandle>,
    finished: BTreeMap<u16, Answer>,
    observations: Vec<Observation>,
}

pub(super) async fn drive(
    input: &Input,
    settings: &Settings,
) -> Result<super::Output, OperationError> {
    let (router, app, admission, in_flight) = build_router(input, settings);
    let mut driver = Driver {
        router,
        admission,
        in_flight,
        specs: input
            .requests
            .iter()
            .map(|spec| (spec.id, spec.clone()))
            .collect(),
        tasks: JoinSet::new(),
        aborts: BTreeMap::new(),
        finished: BTreeMap::new(),
        observations: Vec::new(),
    };
    for step in &input.steps {
        driver.apply(step, &app).await?;
    }
    Ok(super::Output {
        limits: limits_output(settings),
        observations: driver.observations,
    })
}

fn build_router(
    input: &Input,
    settings: &Settings,
) -> (
    Router,
    Arc<App>,
    Arc<AdmissionControl>,
    Arc<InFlightRequests>,
) {
    let app = Arc::new(app::gate_app(input));
    let stack = AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(settings)),
        in_flight: Arc::new(InFlightRequests::default()),
    };
    let inner = Router::new()
        .fallback(app::dispatch)
        .with_state(Arc::clone(&app));
    let stacked = stack.wrap(inner);
    let roots = request_root_paths::Settings {
        server_root_path: String::new(),
        server_root_paths: vec![input.root_path.clone()],
    };
    let router = match RootPaths::from_settings(&roots) {
        Some(paths) => paths.wrap(stacked),
        None => stacked,
    };
    (router, app, stack.admission, stack.in_flight)
}

fn limits_output(settings: &Settings) -> Option<LimitsOutput> {
    let limits = settings.limits?;
    Some(LimitsOutput {
        max_in_flight_requests: u64::try_from(limits.max_in_flight_requests).unwrap_or(u64::MAX),
        max_queued_requests: u64::try_from(limits.max_queued_requests).unwrap_or(u64::MAX),
        queue_timeout_seconds: Number::from_f64(limits.queue_timeout.as_secs_f64())
            .unwrap_or_else(|| Number::from(0_u64)),
    })
}

impl Driver {
    async fn apply(&mut self, step: &Step, app: &Arc<App>) -> Result<(), OperationError> {
        match step {
            Step::Start { request } => self.start(*request),
            Step::AwaitStarted { request } => self.await_started(*request, app).await,
            Step::AwaitQueued { queued } => self.await_queued(*queued).await,
            Step::Release { request } => self.release(*request, app)?,
            Step::Cancel { request } => self.cancel(*request).await?,
            Step::Complete { request } => {
                let answer = self.complete(*request).await?;
                self.observations.push(completed(*request, answer));
            }
            Step::Stats => {
                let snapshot = self.admission.stats();
                self.observations.push(Observation::Stats {
                    admitted: snapshot.admitted,
                    queued: snapshot.queued,
                    rejected_total: snapshot.rejected_total,
                    in_flight: self.in_flight.current.load(Ordering::Acquire),
                });
            }
        }
        Ok(())
    }

    async fn await_started(&mut self, request: u16, app: &Arc<App>) {
        let Ok(path) = self.path(request) else {
            return;
        };
        let Some(gate) = app.gates.get(&path) else {
            return;
        };
        let started = Arc::clone(&gate.started);
        let reached = timeout(WAIT, started.notified()).await.is_ok();
        self.observations.push(if reached {
            Observation::Started { request }
        } else {
            Observation::TimedOut {
                wait: "started".to_owned(),
            }
        });
    }

    async fn await_queued(&mut self, queued: u16) {
        let reached = wait_queued(&self.admission, u64::from(queued)).await;
        self.observations.push(if reached {
            Observation::Queued {
                queued: u64::from(queued),
            }
        } else {
            Observation::TimedOut {
                wait: "queued".to_owned(),
            }
        });
    }

    fn release(&self, request: u16, app: &Arc<App>) -> Result<(), OperationError> {
        let path = self.path(request)?;
        let gate = app
            .gates
            .get(&path)
            .ok_or_else(|| invalid("a step names an unknown request"))?;
        gate.release.notify_one();
        Ok(())
    }

    fn path(&self, request: u16) -> Result<String, OperationError> {
        self.specs
            .get(&request)
            .map(|spec| spec.path.clone())
            .ok_or_else(|| invalid("a step names an unknown request"))
    }

    fn start(&mut self, request: u16) {
        let Ok(path) = self.path(request) else {
            return;
        };
        let router = self.router.clone();
        let abort = self.tasks.spawn(async move {
            let answer = call(router, path).await;
            (request, answer)
        });
        self.aborts.insert(request, abort);
    }

    async fn cancel(&mut self, request: u16) -> Result<(), OperationError> {
        let Some(abort) = self.aborts.remove(&request) else {
            return Ok(());
        };
        abort.abort();
        while let Some(joined) = self.tasks.join_next().await {
            let Ok((finished, answer)) = joined else {
                return Ok(());
            };
            self.finished.insert(finished, answer);
        }
        Ok(())
    }

    async fn complete(&mut self, request: u16) -> Result<Answer, OperationError> {
        if let Some(answer) = self.finished.remove(&request) {
            return Ok(answer);
        }
        while let Some(joined) = self.tasks.join_next().await {
            let Ok((finished, answer)) = joined else {
                return Err(invalid("a request task did not complete"));
            };
            if finished == request {
                return Ok(answer);
            }
            self.finished.insert(finished, answer);
        }
        Err(invalid("a request never completed"))
    }
}

async fn wait_queued(admission: &AdmissionControl, queued: u64) -> bool {
    timeout(WAIT, async {
        while admission.stats().queued != queued {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .is_ok()
}

async fn call(router: Router, path: String) -> Answer {
    let Ok(request) = Request::builder().uri(path).body(Body::empty()) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            HeaderMap::new(),
            Bytes::new(),
        );
    };
    let response = router
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {});
    let status = response.status();
    let headers = response.headers().clone();
    let Ok(body) = to_bytes(response.into_body(), BODY_LIMIT).await else {
        return (status, headers, Bytes::new());
    };
    (status, headers, body)
}

fn completed(request: u16, answer: Answer) -> Observation {
    let (status, headers, body) = answer;
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = header("content-type");
    Observation::Completed {
        request,
        status: status.as_u16(),
        retry_after: header("retry-after"),
        body: body_value(content_type.as_deref(), &body),
        content_type,
    }
}

fn body_value(content_type: Option<&str>, bytes: &[u8]) -> Option<Value> {
    if bytes.is_empty() {
        return None;
    }
    let text = str::from_utf8(bytes).ok()?;
    if content_type.is_some_and(|value| value.starts_with("application/json")) {
        return parse_json(text).ok();
    }
    Some(Value::String(text.to_owned()))
}
