use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_provider::opensandbox::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, RequestTimeout, SandboxHttp, SandboxRequest,
    SandboxStream,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use serde::Serialize;
use serde_json::Value;

use super::helper_error;
use super::{Input, ScriptedResponse};

pub(super) type Environment = BTreeMap<String, String>;
pub(super) type Sandbox = OpenSandbox<RecordingHttp, Environment, ManualClock>;

pub(super) struct RecordingHttp {
    pub(super) responses: Mutex<VecDeque<ScriptedResponse>>,
    pub(super) requests: Mutex<Vec<SandboxRequest>>,
}

pub(super) struct ManualClock {
    pub(super) now: Mutex<Duration>,
}

#[derive(Serialize)]
pub(super) struct RequestOutput {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    json: Option<Value>,
    timeout: TimeoutOutput,
}

#[derive(Serialize)]
struct TimeoutOutput {
    connect: Option<f64>,
    read: Option<f64>,
    write: Option<f64>,
    pool: Option<f64>,
}

#[async_trait]
impl SandboxHttp for RecordingHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let scripted = {
            let mut responses = self
                .responses
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if responses.is_empty() {
                ScriptedResponse {
                    status: 200,
                    body: String::new(),
                }
            } else if responses.len() == 1 {
                responses.front().cloned().unwrap_or_default()
            } else {
                responses.pop_front().unwrap_or_default()
            }
        };
        Ok(SandboxStream {
            status: scripted.status,
            body: stream::iter(vec![Ok(scripted.body.into_bytes())]).boxed(),
        })
    }
}

#[async_trait]
impl PollClock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn sleep(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        let next = now.saturating_add(duration);
        *now = next;
    }
}

impl Default for ScriptedResponse {
    fn default() -> Self {
        Self {
            status: 200,
            body: String::new(),
        }
    }
}

pub(super) fn recording_sandbox(input: &Input) -> Sandbox {
    OpenSandbox {
        http: RecordingHttp {
            responses: Mutex::new(input.scripted()),
            requests: Mutex::new(Vec::new()),
        },
        environment: Environment::new(),
        clock: ManualClock {
            now: Mutex::new(Duration::ZERO),
        },
    }
}

pub(super) fn block_on<Output, Work>(work: Work) -> Result<Output, Box<crate::helpers::HelperError>>
where
    Work: Future<Output = Result<Output, OpenSandboxError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    runtime
        .block_on(work)
        .map_err(|error| helper_error("OpenSandboxError", error))
}

pub(super) fn requests_output(
    http: &RecordingHttp,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let requests = http
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut outputs = Vec::with_capacity(requests.len());
    for request in &requests {
        outputs.push(request_output(request)?);
    }
    serde_json::to_value(outputs).map_err(|error| helper_error("BodyEncoding", error))
}

pub(super) fn request_output(
    request: &SandboxRequest,
) -> Result<RequestOutput, Box<crate::helpers::HelperError>> {
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
    Ok(RequestOutput {
        method: request.method.as_str().to_owned(),
        url: request.url.to_string(),
        headers,
        json,
        timeout: timeout_output(request.timeout),
    })
}

pub(super) fn single_request(
    http: &RecordingHttp,
) -> Result<SandboxRequest, Box<crate::helpers::HelperError>> {
    let requests = http.requests.lock().unwrap_or_else(PoisonError::into_inner);
    match (requests.first(), requests.len()) {
        (Some(request), 1) => Ok(request.clone()),
        _ => Err(helper_error("UnexpectedRequests", requests.len())),
    }
}

fn timeout_output(timeout: Option<RequestTimeout>) -> TimeoutOutput {
    let fields = timeout.unwrap_or(RequestTimeout {
        connect: None,
        read: None,
        write: None,
        pool: None,
    });
    TimeoutOutput {
        connect: fields.connect.as_ref().map(Duration::as_secs_f64),
        read: fields.read.as_ref().map(Duration::as_secs_f64),
        write: fields.write.as_ref().map(Duration::as_secs_f64),
        pool: fields.pool.as_ref().map(Duration::as_secs_f64),
    }
}
