#![cfg(test)]

use std::collections::VecDeque;
use std::future::ready;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::Stream;
use futures_util::stream::{iter, unfold};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::assistants::client::ClientOptions;
use crate::assistants::transport::AssistantsTransport;
use crate::thread_runs::error::ThreadRunsError;
use crate::thread_runs::operations::RunsApi;
use crate::thread_runs::records::RunEvent;
use crate::thread_runs::stream::{RunChunks, RunEventHandler, RunStreamTransport};

pub(super) const KEY: &str = "sk-fixture";
pub(super) const BASE: &str = "https://example.test/v1";
pub(super) const QUEUED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_a","status":"queued","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;
pub(super) const IN_PROGRESS: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"started_at":1700000001,"assistant_id":"asst_1","thread_id":"thread_a","status":"in_progress","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;
pub(super) const COMPLETED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"completed_at":1700000005,"assistant_id":"asst_1","thread_id":"thread_a","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"metadata":{"tag":"x"},"response_format":"auto","tool_choice":"auto","tools":[{"type":"code_interpreter"}],"truncation_strategy":{"type":"auto"},"usage":{"completion_tokens":10,"prompt_tokens":5,"total_tokens":15},"temperature":0.5,"top_p":0.25}"#;
pub(super) const REQUIRES_ACTION: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"assistant_id":"asst_1","thread_id":"thread_a","status":"requires_action","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"required_action":{"submit_tool_outputs":{"tool_calls":[{"id":"call_1","function":{"arguments":"{}","name":"lookup"},"type":"function"}]},"type":"submit_tool_outputs"},"tools":[{"type":"code_interpreter"}]}"#;
pub(super) const FAILED: &str = r#"{"id":"run_1","object":"thread.run","created_at":1700000000,"failed_at":1700000005,"assistant_id":"asst_1","thread_id":"thread_a","status":"failed","instructions":"base","last_error":{"code":"server_error","message":"boom"},"model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[{"type":"code_interpreter"}]}"#;

// What the fake transport saw of one request, with the secret headers read back as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) deadline: Duration,
    pub(super) accept: Option<String>,
    pub(super) authorization: Option<String>,
    pub(super) organization: Option<String>,
    pub(super) beta: Option<String>,
    pub(super) poll_helper: Option<String>,
    pub(super) stream_helper: Option<String>,
    pub(super) custom_handler: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) body: Option<Value>,
}

#[derive(Default)]
pub(super) struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, HeaderMap, Vec<u8>)>>,
    gate: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    prepared: Mutex<Option<RunChunks>>,
}

struct GateChunks {
    receiver: mpsc::Receiver<Vec<u8>>,
}

impl Stream for GateChunks {
    type Item = Result<Vec<u8>, ThreadRunsError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut()
            .receiver
            .poll_recv(cx)
            .map(|chunk| chunk.map(Ok))
    }
}

impl Recorder {
    pub(super) fn queue(&self, status: u16, headers: &[(&str, &str)], body: &str) {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            let header = HeaderName::try_from(*name).unwrap();
            map.insert(header, HeaderValue::try_from(*value).unwrap());
        }
        self.replies
            .lock()
            .unwrap()
            .push_back((status, map, body.as_bytes().to_vec()));
    }

    // A header value that is not visible ASCII is legal in a response but unreadable as text.
    pub(super) fn queue_opaque_header(
        &self,
        status: u16,
        name: &'static str,
        value: &[u8],
        body: &str,
    ) {
        let mut headers = HeaderMap::new();
        let header = HeaderValue::from_bytes(value).unwrap();
        headers.insert(HeaderName::from_static(name), header);
        self.replies
            .lock()
            .unwrap()
            .push_back((status, headers, body.as_bytes().to_vec()));
    }

    // The stream is fed by the caller through this channel, so a handler can release the next chunk.
    pub(super) fn gate(&self) -> mpsc::Sender<Vec<u8>> {
        let (sender, receiver) = mpsc::channel(1);
        *self.gate.lock().unwrap() = Some(receiver);
        sender
    }

    // A prepared chunk stream, so a case can script delays or errors between chunks.
    pub(super) fn stream_chunks(&self, chunks: RunChunks) {
        *self.prepared.lock().unwrap() = Some(chunks);
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, request: &OutboundRequest) {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => Some(serde_json::from_slice(bytes).unwrap()),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => None,
        };
        let content_type = header_of(&request.headers, "content-type")
            .or_else(|| body.is_some().then(|| String::from("application/json")));
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
            deadline: request.deadline,
            accept: header_of(&request.headers, "accept"),
            authorization: header_of(&request.headers, "authorization"),
            organization: header_of(&request.headers, "openai-organization"),
            beta: header_of(&request.headers, "openai-beta"),
            poll_helper: header_of(&request.headers, "x-stainless-poll-helper"),
            stream_helper: header_of(&request.headers, "x-stainless-stream-helper"),
            custom_handler: header_of(&request.headers, "x-stainless-custom-event-handler"),
            content_type,
            body,
        });
    }

    fn next_reply(&self) -> (u16, HeaderMap, Vec<u8>) {
        self.replies.lock().unwrap().pop_front().unwrap_or_else(|| {
            (
                500,
                HeaderMap::new(),
                b"{\"error\":{\"message\":\"the fixture queues no reply\"}}".to_vec(),
            )
        })
    }
}

fn header_of(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.to_str().unwrap().to_owned())
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&request);
        let (status, headers, body) = self.next_reply();
        ready(Ok(UpstreamResponse {
            status,
            headers,
            body,
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        self.record(&request);
        let prepared = self.prepared.lock().unwrap().take();
        if let Some(chunks) = prepared {
            return ready(Ok(chunks));
        }
        let gated = self.gate.lock().unwrap().take();
        if let Some(receiver) = gated {
            let chunks: RunChunks = Box::pin(GateChunks { receiver });
            return ready(Ok(chunks));
        }
        let (status, _headers, body) = self.next_reply();
        if status >= 400 {
            return ready(Err(ThreadRunsError::Status(status, body)));
        }
        let chunks: RunChunks = Box::pin(iter(vec![Ok(body)]));
        ready(Ok(chunks))
    }
}

#[derive(Default)]
pub(super) struct RecordingHandler {
    pub(super) calls: Mutex<Vec<String>>,
}

impl RecordingHandler {
    pub(super) fn take(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

fn error_kind(error: &ThreadRunsError) -> &'static str {
    if matches!(error, ThreadRunsError::Timeout) {
        return "timeout";
    }
    if matches!(error, ThreadRunsError::ProviderError { .. }) {
        return "stream_error";
    }
    if matches!(error, ThreadRunsError::Status { .. }) {
        return "status";
    }
    "other"
}

impl RunEventHandler for RecordingHandler {
    fn on_event(&self, event: &RunEvent) {
        self.calls.lock().unwrap().push(event.name().to_owned());
    }

    fn on_exception(&self, error: &ThreadRunsError) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("exception:{}", error_kind(error)));
    }

    fn on_timeout(&self) {
        self.calls.lock().unwrap().push(String::from("timeout"));
    }

    fn on_end(&self) {
        self.calls.lock().unwrap().push(String::from("end"));
    }
}

// Releases one prepared chunk when the first event reaches the handler.
pub(super) struct GatedHandler {
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) sender: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pub(super) payload: Vec<u8>,
}

impl GatedHandler {
    pub(super) fn take(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl RunEventHandler for GatedHandler {
    fn on_event(&self, event: &RunEvent) {
        self.calls.lock().unwrap().push(event.name().to_owned());
        let pending = self.sender.lock().unwrap().take();
        if let Some(queued) = pending {
            queued.try_send(self.payload.clone()).unwrap();
            self.calls.lock().unwrap().push(String::from("released"));
        }
    }

    fn on_exception(&self, error: &ThreadRunsError) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("exception:{}", error_kind(error)));
    }

    fn on_timeout(&self) {
        self.calls.lock().unwrap().push(String::from("timeout"));
    }

    fn on_end(&self) {
        self.calls.lock().unwrap().push(String::from("end"));
    }
}

pub(super) fn options(key: &str) -> ClientOptions {
    ClientOptions {
        api_key: Some(SecretString::from(key)),
        api_base: Some(String::from(BASE)),
        timeout: None,
        max_retries: None,
        organization: Some(String::from("org-fixture")),
    }
}

pub(super) fn harness() -> (RunsApi<Recorder>, Arc<Recorder>) {
    let transport = Arc::new(Recorder::default());
    let api = RunsApi {
        transport: Arc::clone(&transport),
    };
    (api, transport)
}

pub(super) fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

pub(super) fn parts_chunks(parts: Vec<Result<Vec<u8>, ThreadRunsError>>) -> RunChunks {
    Box::pin(iter(parts))
}

// Every part after the first waits the delay, so a paused clock can race it against a read timeout.
pub(super) fn trickle_chunks(parts: Vec<Vec<u8>>, delay: Duration) -> RunChunks {
    let stream = unfold(
        (VecDeque::from(parts), false),
        move |(mut queued, started)| async move {
            if started {
                sleep(delay).await;
            }
            queued
                .pop_front()
                .map(|part| (Ok::<Vec<u8>, ThreadRunsError>(part), (queued, true)))
        },
    );
    Box::pin(stream)
}

// The request every case expects: the SDK's 600s default, JSON accept, bearer key, organization and the assistants beta header.
pub(super) fn wire(method: &str, url: &str, body: Option<&str>) -> Seen {
    Seen {
        method: method.to_owned(),
        url: url.to_owned(),
        deadline: Duration::from_secs(600),
        accept: None,
        authorization: Some(format!("Bearer {KEY}")),
        organization: Some(String::from("org-fixture")),
        beta: Some(String::from("assistants=v2")),
        poll_helper: None,
        stream_helper: None,
        custom_handler: None,
        content_type: (method != "GET").then(|| String::from("application/json")),
        body: body.map(parse),
    }
}
