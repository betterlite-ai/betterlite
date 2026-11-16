#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::mock::MockError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, base_builder};
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_openai::videos::reply::VideoObject;
use http::header::HeaderMap;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::time::sleep;
use url::Url;

use crate::api::fal::video_prepare::VideoHeaders;
use crate::api::fal::video_results::UpstreamReply;
use crate::api::fal::video_results::content::content_response;
use crate::api::fal::video_results::error::VideoResultsError;
use crate::api::fal::video_results::status::{StatusCall, StatusRequest, status_response};
use crate::api::fal::video_results::transport::{
    ClientKind, SuppliedTransport, VideoClients, VideoTransport,
};

pub(crate) const STATUS_URL: &str =
    "https://queue.fal.run/fal-ai/kling-video/requests/req-1/status";
pub(crate) const MODEL_PATH: &str = "fal-ai/kling-video";
pub(crate) const SHORT_READ_LIMIT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(500),
    ..TransportLimits::DEFAULT
};
pub(crate) const SMALL_BODY_LIMIT: TransportLimits = TransportLimits {
    body_bytes: 4,
    ..TransportLimits::DEFAULT
};
const HOLD: Duration = Duration::from_secs(2);

pub(crate) type RecordedRequest = (String, BTreeMap<String, String>);

pub(crate) enum Step {
    Answer(u16, Vec<u8>),
    Fail(TransportError),
}

pub(crate) struct FakeTransport {
    script: Mutex<VecDeque<Step>>,
    recorded: Mutex<Vec<RecordedRequest>>,
}

impl FakeTransport {
    pub(crate) fn new(script: Vec<Step>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(script)),
            recorded: Mutex::default(),
        }
    }

    pub(crate) fn idle() -> Self {
        Self::new(Vec::new())
    }

    pub(crate) fn recorded(&self) -> Vec<RecordedRequest> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, url: &Url, headers: &HeaderMap) {
        let values = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect();
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.as_str().to_owned(), values));
    }
}

impl VideoTransport for FakeTransport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(&url, &headers);
        let step = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test scripts one answer per request");
        std::future::ready(match step {
            Step::Answer(status, body) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Step::Fail(source) => Err(source),
        })
    }
}

pub(crate) fn answered(status: u16, body: &str) -> Vec<Step> {
    vec![Step::Answer(status, body.as_bytes().to_vec())]
}

pub(crate) fn outbound_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the proxy policy"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits).expect("the client")
}

// keeps the connection open without answering, so only the client's read limit can end the request
pub(crate) async fn hold_open(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(
        read, 0,
        "the client sends its request before the read limit"
    );
    sleep(HOLD).await;
}

// reads the client's request and answers it with the fixed reply the caller scripted
pub(crate) async fn serve_artifact(listener: TcpListener, answer: &'static [u8]) {
    let (mut stream, _) = listener.accept().await.expect("the connection");
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.expect("the request bytes");
    assert_ne!(read, 0, "the client sends its request before the reply");
    stream.write_all(answer).await.expect("the reply");
}

pub(crate) fn mock_failure() -> Step {
    let source = serde_json::from_str::<u32>("not a number").expect_err("invalid JSON");
    Step::Fail(TransportError::Mock(MockError::Encode(source)))
}

pub(crate) fn headers(entries: &[(&str, &str)]) -> VideoHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn request_headers() -> VideoHeaders {
    headers(&[
        ("Authorization", "Bearer fal-key"),
        ("Host", "queue.fal.run"),
        ("Content-Length", "0"),
        ("Transfer-Encoding", "identity"),
        ("X-Trace", "t-1"),
    ])
}

pub(crate) fn status_request() -> StatusRequest {
    StatusRequest {
        url: Url::parse(STATUS_URL).expect("the status url parses"),
        headers: request_headers(),
    }
}

pub(crate) fn status_reply(body: &str) -> UpstreamReply {
    UpstreamReply {
        status: 200,
        headers: VideoHeaders::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn content_reply(status: u16, headers: VideoHeaders, body: &[u8]) -> UpstreamReply {
    UpstreamReply {
        status,
        headers,
        body: body.to_vec(),
    }
}

pub(crate) fn run_status(
    transport: &FakeTransport,
    kind: ClientKind,
    call: &StatusCall<'_>,
) -> Result<VideoObject, VideoResultsError> {
    let clients = VideoClients {
        sync: transport,
        asynchronous: transport,
    };
    block_on_status(&clients, kind, None, call)
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime builds")
}

pub(crate) fn block_on_status(
    clients: &VideoClients<'_, FakeTransport>,
    kind: ClientKind,
    supplied: Option<&SuppliedTransport<'_, FakeTransport>>,
    call: &StatusCall<'_>,
) -> Result<VideoObject, VideoResultsError> {
    runtime().block_on(status_response(clients, kind, supplied, call))
}

pub(crate) fn run_content(
    transport: &FakeTransport,
    kind: ClientKind,
    reply: &UpstreamReply,
) -> Result<Vec<u8>, VideoResultsError> {
    let clients = VideoClients {
        sync: transport,
        asynchronous: transport,
    };
    runtime().block_on(content_response(&clients, kind, reply))
}

pub(crate) fn decoded(video: &VideoObject) -> (Option<String>, Option<String>, String) {
    let id = decode_video_id_with_provider(&video.id);
    (id.custom_llm_provider, id.model_id, id.video_id)
}

pub(crate) fn message(video: &VideoObject) -> Option<String> {
    let error = video.error.as_ref()?;
    error
        .get("message")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}
