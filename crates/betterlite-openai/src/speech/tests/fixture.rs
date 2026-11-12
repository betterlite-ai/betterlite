use std::future::{Future, ready};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::files::transport::{FileTransport, StreamReply};
use crate::speech::client::SpeechConnection;
use crate::speech::logging::{SpeechLogRecord, SpeechLogger};

#[derive(Debug, Clone, Default)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) chunks: Vec<Vec<u8>>,
}

impl Reply {
    pub(super) fn ok(content_type: &str, chunks: Vec<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![
                (String::from("content-type"), content_type.to_owned()),
                (String::from("x-request-id"), String::from("req-1")),
            ],
            chunks,
        }
    }
}

// One recorded upstream answer; the request the production code sent is kept for the assertions
pub(super) struct Fixture {
    reply: Reply,
    headers: HeaderMap,
    seen: OnceLock<OutboundRequest>,
    sends: AtomicUsize,
    broken: bool,
}

impl Fixture {
    pub(super) fn new(reply: Reply) -> Self {
        let mut headers = HeaderMap::new();
        for (name, value) in &reply.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).expect("the fixture header name is valid"),
                HeaderValue::from_str(value).expect("the fixture header value is valid"),
            );
        }
        Self {
            reply,
            headers,
            seen: OnceLock::new(),
            sends: AtomicUsize::new(0),
            broken: false,
        }
    }

    pub(super) fn broken() -> Self {
        let mut fixture = Self::new(Reply::default());
        fixture.broken = true;
        fixture
    }

    pub(super) fn request(&self) -> &OutboundRequest {
        self.seen.get().expect("the call reached the transport")
    }

    pub(super) fn sends(&self) -> usize {
        self.sends.load(Ordering::Relaxed)
    }

    fn record(&self, request: OutboundRequest) {
        drop(self.seen.set(request));
        let _previous = self.sends.fetch_add(1, Ordering::Relaxed);
    }

    fn stream_reply(&self) -> Result<StreamReply, TransportError> {
        if self.broken {
            return Err(TransportError::Closed);
        }
        Ok(StreamReply {
            status: self.reply.status,
            headers: self.headers.clone(),
            body: stream::iter(self.reply.chunks.clone().into_iter().map(Ok)).boxed(),
        })
    }

    fn buffered_reply(&self) -> Result<UpstreamResponse, TransportError> {
        if self.broken {
            return Err(TransportError::Closed);
        }
        let mut body = Vec::new();
        for chunk in &self.reply.chunks {
            body.extend_from_slice(chunk);
        }
        Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.headers.clone(),
            body,
        })
    }
}

impl FileTransport for Fixture {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        let reply = self.buffered_reply();
        async move { reply }
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.record(request);
        ready(self.stream_reply())
    }
}

#[derive(Debug, Default)]
pub(super) struct Recorder {
    pub(super) records: Vec<Recorded>,
}

#[derive(Debug)]
pub(super) struct Recorded {
    pub(super) input: String,
    pub(super) api_base: String,
    pub(super) body: IndexMap<String, Json>,
}

impl SpeechLogger for Recorder {
    fn pre_call(&mut self, record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        self.records.push(Recorded {
            input: record.input.to_owned(),
            api_base: record.api_base.to_owned(),
            body: record.body,
        });
        Ok(())
    }
}

pub(super) fn connection() -> SpeechConnection {
    SpeechConnection {
        api_key: Some(SecretString::from("sk-speech")),
        api_base: Some(String::from("https://speech.test/v1")),
        timeout: Duration::from_secs(30),
        max_retries: 4,
        organization: Some(String::from("org-not-forwarded")),
        project: Some(String::from("project-not-forwarded")),
        environment_organization: None,
        environment_project: None,
        client: None,
        shared_session: None,
    }
}

pub(super) fn sent_body(fixture: &Fixture) -> serde_json::Value {
    match &fixture.request().body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).expect("the sent body is JSON"),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            panic!("the speech call sends a JSON body")
        }
    }
}

pub(super) fn header(fixture: &Fixture, name: &str) -> Option<String> {
    fixture
        .request()
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
