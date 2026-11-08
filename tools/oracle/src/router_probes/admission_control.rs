use std::collections::{BTreeMap, BTreeSet};
use std::str;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::Response;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::admission_control::Settings;
use betterlite_config::sections::request_root_paths;
use betterlite_server::admission_control::in_flight::InFlightRequests;
use betterlite_server::admission_control::{AdmissionControl, AdmissionStack};
use betterlite_server::middleware::request_root_paths::RootPaths;
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, from_str as parse_json};
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::Notify;
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::{sleep, timeout};
use tower::ServiceExt as _;

use crate::router_probes::ProbeOutput;
use crate::router_probes::ProbeOutput::AdmissionControl as AdmissionOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const WAIT: Duration = Duration::from_secs(5);
const BODY_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
    #[serde(default)]
    pub root_path: String,
    pub requests: Vec<RequestSpec>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSpec {
    pub id: u16,
    pub path: String,
    pub behavior: Behavior,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Behavior {
    Immediate,
    Hold,
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Start { request: u16 },
    AwaitStarted { request: u16 },
    AwaitQueued { queued: u16 },
    Release { request: u16 },
    Cancel { request: u16 },
    Complete { request: u16 },
    Stats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub limits: Option<LimitsOutput>,
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsOutput {
    pub max_in_flight_requests: u64,
    pub max_queued_requests: u64,
    pub queue_timeout_seconds: Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Observation {
    Started {
        request: u16,
    },
    Queued {
        queued: u64,
    },
    Stats {
        admitted: u64,
        queued: u64,
        rejected_total: u64,
        in_flight: u64,
    },
    Completed {
        request: u16,
        status: u16,
        retry_after: Option<String>,
        content_type: Option<String>,
        body: Option<Value>,
    },
    TimedOut {
        wait: String,
    },
}

type Answer = (StatusCode, HeaderMap, Bytes);

struct Gate {
    behavior: Behavior,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

struct App {
    gates: BTreeMap<String, Gate>,
}

async fn dispatch(State(app): State<Arc<App>>, OriginalUri(uri): OriginalUri) -> Response {
    let Some(gate) = app.gates.get(uri.path()) else {
        return Response::new(Body::from("ok"));
    };
    match gate.behavior {
        Behavior::Immediate => {
            gate.started.notify_one();
            Response::new(Body::from("ok"))
        }
        Behavior::Hold => {
            gate.started.notify_one();
            gate.release.notified().await;
            Response::new(Body::from("ok"))
        }
        Behavior::Stream => {
            gate.started.notify_one();
            let notifier = Arc::clone(&gate.release);
            let body = Body::from_stream(stream::unfold(0_u8, move |step| {
                stream_chunk(step, Arc::clone(&notifier))
            }));
            Response::new(body)
        }
    }
}

async fn stream_chunk(step: u8, release: Arc<Notify>) -> Option<(Result<Bytes, axum::Error>, u8)> {
    match step {
        0 => Some((Ok(Bytes::from_static(b"first")), 1)),
        1 => {
            release.notified().await;
            Some((Ok(Bytes::from_static(b"last")), 2))
        }
        _ => None,
    }
}

struct Driver {
    router: Router,
    admission: Arc<AdmissionControl>,
    in_flight: Arc<InFlightRequests>,
    specs: BTreeMap<u16, RequestSpec>,
    tasks: JoinSet<(u16, Answer)>,
    aborts: BTreeMap<u16, AbortHandle>,
    finished: BTreeMap<u16, Answer>,
    observations: Vec<Observation>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let ids = validate_requests(input)?;
    validate_steps(input, &ids)?;
    let settings = load_settings(&input.settings)?;
    let runtime = RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| OperationError::InvalidRequest {
            message: format!("the probe runtime could not start: {error}"),
        })?;
    runtime
        .block_on(drive(input, &settings))
        .map(AdmissionOutput)
}

fn validate_requests(input: &Input) -> Result<BTreeSet<u16>, OperationError> {
    let mut paths = BTreeSet::<&str>::new();
    let mut ids = BTreeSet::<u16>::new();
    for spec in &input.requests {
        if !paths.insert(spec.path.as_str()) {
            return Err(invalid("a request path is used twice"));
        }
        if !ids.insert(spec.id) {
            return Err(invalid("a request id is used twice"));
        }
    }
    Ok(ids)
}

fn validate_steps(input: &Input, ids: &BTreeSet<u16>) -> Result<(), OperationError> {
    let mut started = BTreeSet::<u16>::new();
    let mut finished = BTreeSet::<u16>::new();
    for step in &input.steps {
        match step {
            Step::Start { request } => {
                if !ids.contains(request) {
                    return Err(invalid("a step names an unknown request"));
                }
                if !started.insert(*request) {
                    return Err(invalid("a request is started twice"));
                }
            }
            Step::AwaitQueued { .. } | Step::Stats => {}
            Step::AwaitStarted { request }
            | Step::Release { request }
            | Step::Cancel { request }
            | Step::Complete { request } => {
                if !ids.contains(request) {
                    return Err(invalid("a step names an unknown request"));
                }
                if !started.contains(request) {
                    return Err(invalid("a step runs before its request starts"));
                }
                if matches!(step, Step::Cancel { .. } | Step::Complete { .. })
                    && !finished.insert(*request)
                {
                    return Err(invalid("a request terminates twice"));
                }
            }
        }
    }
    if started.len() != finished.len() {
        return Err(invalid("a started request never completes or cancels"));
    }
    Ok(())
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

fn load_settings(settings: &BTreeMap<String, Value>) -> Result<Settings, OperationError> {
    #[derive(Serialize)]
    struct ConfigView<'settings> {
        general_settings: &'settings BTreeMap<String, Value>,
    }
    let source = serde_saphyr::to_string(&ConfigView {
        general_settings: settings,
    })
    .map_err(|error| invalid(&format!("the probe settings are not valid YAML: {error}")))?;
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .map_err(|error| {
        invalid(&format!(
            "the probe settings are not a valid config: {error}"
        ))
    })?;
    Ok(config.sections.admission_control)
}

fn gate_app(input: &Input) -> App {
    let mut gates = BTreeMap::new();
    for spec in &input.requests {
        gates.insert(
            spec.path.clone(),
            Gate {
                behavior: spec.behavior,
                started: Arc::new(Notify::new()),
                release: Arc::new(Notify::new()),
            },
        );
    }
    App { gates }
}

async fn drive(input: &Input, settings: &Settings) -> Result<Output, OperationError> {
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
    Ok(Output {
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
    let app = Arc::new(gate_app(input));
    let stack = AdmissionStack {
        admission: Arc::new(AdmissionControl::from_settings(settings)),
        in_flight: Arc::new(InFlightRequests::default()),
    };
    let inner = Router::new()
        .fallback(dispatch)
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
            Step::Complete { request } => self.complete_step(*request).await?,
            Step::Stats => self.push_stats(),
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

    async fn complete_step(&mut self, request: u16) -> Result<(), OperationError> {
        let answer = self.complete(request).await?;
        self.observations.push(completed(request, answer));
        Ok(())
    }

    fn push_stats(&mut self) {
        let snapshot = self.admission.stats();
        self.observations.push(Observation::Stats {
            admitted: snapshot.admitted,
            queued: snapshot.queued,
            rejected_total: snapshot.rejected_total,
            in_flight: self.in_flight.current.load(Ordering::Acquire),
        });
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
