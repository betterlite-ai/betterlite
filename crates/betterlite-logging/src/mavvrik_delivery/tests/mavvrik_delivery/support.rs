use std::collections::VecDeque;
use std::future::{Future, ready};
use std::mem;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use betterlite_logging::focus_records::destinations::FocusTimeWindow;
use betterlite_logging::mavvrik_delivery::destination::{FocusMavvrikDestination, MavvrikSettings};
use betterlite_logging::mavvrik_delivery::transport::{
    Method, Transport, TransportRequest, TransportResponse,
};
use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

pub(super) const API_ROOT: &str = "https://api.mavvrik.dev/tenant";
pub(super) const AGENT_URL: &str = "https://api.mavvrik.dev/tenant/metrics/agent/ai/conn-1";
pub(super) const SIGNED_URL: &str =
    "https://storage.googleapis.com/bucket/object?X-Goog-Signature=abc";
pub(super) const SESSION_URI: &str = "https://storage.googleapis.com/upload/session-1";
pub(super) const WINDOW_EPOCH: i64 = 1_709_596_800;

#[derive(Debug, Error)]
pub(super) enum ScriptedError {
    #[error("the request timed out")]
    Timeout,
}

#[derive(Debug)]
pub(super) enum Step {
    Reply(TransportResponse),
    Timeout,
}

#[derive(Debug)]
pub(super) struct Seen {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) headers: Vec<(&'static str, String)>,
    pub(super) api_key: Option<String>,
    pub(super) body: Vec<u8>,
    pub(super) timeout_secs: u64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ScriptedTransport {
    steps: Arc<Mutex<VecDeque<Step>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl ScriptedTransport {
    pub(super) fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(VecDeque::from(steps))),
            seen: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<(Method, String)> {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .map(|call| (call.method, call.url.clone()))
            .collect()
    }

    pub(super) fn drain(&self) -> Vec<Seen> {
        mem::take(&mut *self.seen.lock().unwrap())
    }

    pub(super) fn methods(&self) -> Vec<Method> {
        self.calls().into_iter().map(|(method, _)| method).collect()
    }
}

impl Transport for ScriptedTransport {
    type Error = ScriptedError;

    fn send(
        &self,
        request: TransportRequest,
    ) -> impl Future<Output = Result<TransportResponse, ScriptedError>> + Send {
        self.seen.lock().unwrap().push(Seen {
            method: request.method,
            url: request.url,
            headers: request.headers,
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            body: request.body,
            timeout_secs: request.timeout.as_secs(),
        });
        let step = self.steps.lock().unwrap().pop_front().unwrap();
        ready(match step {
            Step::Reply(response) => Ok(response),
            Step::Timeout => Err(ScriptedError::Timeout),
        })
    }
}

pub(super) fn respond(status: u16, body: &str) -> Step {
    Step::Reply(TransportResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    })
}

pub(super) fn respond_with_location(status: u16, location: &str) -> Step {
    Step::Reply(TransportResponse {
        status,
        headers: vec![("location".to_owned(), location.to_owned())],
        body: Vec::new(),
    })
}

pub(super) fn registered() -> Step {
    respond(200, r#"{"metricsMarker": 1700000000}"#)
}

pub(super) fn signed_url_found() -> Step {
    respond(200, &format!(r#"{{"url": "{SIGNED_URL}"}}"#))
}

pub(super) fn settings(endpoint: &str) -> MavvrikSettings {
    MavvrikSettings {
        api_key: Some(SecretString::from("key-123")),
        api_endpoint: Some(endpoint.to_owned()),
        connection_id: Some("conn-1".to_owned()),
    }
}

pub(super) fn destination(
    steps: Vec<Step>,
) -> (
    FocusMavvrikDestination<ScriptedTransport>,
    ScriptedTransport,
) {
    let transport = ScriptedTransport::new(steps);
    let destination = FocusMavvrikDestination::new(transport.clone(), settings(API_ROOT)).unwrap();
    (destination, transport)
}

pub(super) fn window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: DateTime::from_timestamp(WINDOW_EPOCH, 0).unwrap(),
        end_time: DateTime::from_timestamp(WINDOW_EPOCH + 86_400, 0).unwrap(),
        frequency: "daily".to_owned(),
    }
}

pub(super) fn block_on<Task, Output>(task: Task) -> Output
where
    Task: Future<Output = Output>,
{
    let mut pinned = pin!(task);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = pinned.as_mut().poll(&mut context) {
            return output;
        }
    }
}

pub(super) fn noise(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut bytes = Vec::with_capacity(length.saturating_add(8));
    while bytes.len() < length {
        state ^= state << 13_u32;
        state ^= state >> 7_u32;
        state ^= state << 17_u32;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(length);
    bytes
}
