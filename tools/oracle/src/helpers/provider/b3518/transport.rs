use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_provider::opensandbox::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, RequestTimeout, SandboxHttp, SandboxRequest,
    SandboxStream,
};
use betterlite_provider::opensandbox::lifecycle::{LifecycleError, SandboxHandle};
use futures_util::StreamExt as _;
use futures_util::stream;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::Value;

use super::{Input, ScriptedResponse, helper_error};

pub(super) type Environment = BTreeMap<String, String>;
pub(super) type Sandbox = OpenSandbox<RecordingHttp, Environment, StepClock>;

#[derive(Default)]
struct State {
    replies: VecDeque<ScriptedResponse>,
    requests: Vec<SandboxRequest>,
}

pub(super) struct RecordingHttp {
    state: Mutex<State>,
}

pub(super) struct StepClock {
    now: Mutex<Duration>,
}

impl Default for ScriptedResponse {
    fn default() -> Self {
        Self {
            status: 200,
            body: String::new(),
        }
    }
}

#[async_trait]
impl SandboxHttp for RecordingHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError> {
        let (status, body) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.requests.push(request);
            let reply = if state.replies.is_empty() {
                ScriptedResponse::default()
            } else if state.replies.len() == 1 {
                state.replies.front().cloned().unwrap_or_default()
            } else {
                state.replies.pop_front().unwrap_or_default()
            };
            (reply.status, reply.body)
        };
        Ok(SandboxStream {
            status,
            body: stream::iter(vec![Ok(body.into_bytes())]).boxed(),
        })
    }
}

#[async_trait]
impl PollClock for StepClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn sleep(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(duration);
    }
}

pub(super) fn recording_sandbox(input: &Input) -> Sandbox {
    OpenSandbox {
        http: RecordingHttp {
            state: Mutex::new(State {
                replies: VecDeque::from(input.responses.clone()),
                requests: Vec::new(),
            }),
        },
        environment: Environment::new(),
        clock: StepClock {
            now: Mutex::new(Duration::ZERO),
        },
    }
}

pub(super) fn block_on<Output, Work>(work: Work) -> Result<Output, Box<crate::helpers::HelperError>>
where
    Work: Future<Output = Result<Output, LifecycleError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    runtime
        .block_on(work)
        .map_err(|error| helper_error("LifecycleError", error))
}

pub(super) fn handle_output(handle: &SandboxHandle) -> Value {
    serde_json::json!({
        "id": &handle.container.id,
        "provider": &handle.container.provider,
        "domain": &handle.container.domain,
        "hidden_params": {
            "api_base": &handle.container.hidden_params.api_base,
            "api_key": handle.container.hidden_params.api_key.as_ref().map(|key| key.expose_secret().to_owned()),
            "execd_endpoint": &handle.execd_endpoint,
            "execd_headers": handle
                .execd_headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect::<BTreeMap<String, String>>(),
            "use_server_proxy": handle.use_server_proxy,
        },
    })
}

#[derive(Serialize)]
struct RequestRecord {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    json: Option<Value>,
    timeout: TimeoutRecord,
}

#[derive(Serialize)]
struct TimeoutRecord {
    connect: Option<f64>,
    read: Option<f64>,
    write: Option<f64>,
    pool: Option<f64>,
}

pub(super) fn requests_output(
    http: &RecordingHttp,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let requests = {
        let state = http.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.requests.clone()
    };
    let mut records = Vec::with_capacity(requests.len());
    for request in &requests {
        records.push(record_request(request)?);
    }
    serde_json::to_value(records).map_err(|error| helper_error("BodyEncoding", error))
}

fn record_request(
    request: &SandboxRequest,
) -> Result<RequestRecord, Box<crate::helpers::HelperError>> {
    let mut headers = BTreeMap::new();
    for (name, value) in &request.headers {
        let text = value
            .to_str()
            .map_err(|error| helper_error("HeaderEncoding", error))?;
        headers.insert(name.as_str().to_owned(), text.to_owned());
    }
    let json = if request.body.is_empty() {
        None
    } else {
        Some(
            serde_json::from_slice(&request.body)
                .map_err(|error| helper_error("RequestBodyEncoding", error))?,
        )
    };
    Ok(RequestRecord {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        headers,
        json,
        timeout: timeout_record(request.timeout),
    })
}

fn timeout_record(timeout: Option<RequestTimeout>) -> TimeoutRecord {
    let fields = timeout.unwrap_or(RequestTimeout {
        connect: None,
        read: None,
        write: None,
        pool: None,
    });
    TimeoutRecord {
        connect: fields.connect.as_ref().map(Duration::as_secs_f64),
        read: fields.read.as_ref().map(Duration::as_secs_f64),
        write: fields.write.as_ref().map(Duration::as_secs_f64),
        pool: fields.pool.as_ref().map(Duration::as_secs_f64),
    }
}
