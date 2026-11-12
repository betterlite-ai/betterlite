use std::future::{Future, ready};
use std::sync::OnceLock;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::speech::logging::{SpeechLogRecord, SpeechLogger};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use indexmap::IndexMap;
use serde_json::{Value, json};
use thiserror::Error;

use super::input::Reply;

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error("a fixture reply header name is not valid")]
    HeaderName {
        #[source]
        source: InvalidHeaderName,
    },
    #[error("a fixture reply header value is not valid")]
    HeaderValue {
        #[source]
        source: InvalidHeaderValue,
    },
}

// One recorded upstream answer; the request the production code sent is kept for the comparison
pub(super) struct Fixture {
    status: u16,
    headers: HeaderMap,
    chunks: Vec<Vec<u8>>,
    seen: OnceLock<OutboundRequest>,
}

impl Fixture {
    pub(super) fn new(reply: Option<&Reply>) -> Result<Self, FixtureError> {
        let absent = Reply::default();
        let given = reply.unwrap_or(&absent);
        let mut headers = HeaderMap::new();
        for (name, value) in &given.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes())
                    .map_err(|source| FixtureError::HeaderName { source })?,
                HeaderValue::from_str(value)
                    .map_err(|source| FixtureError::HeaderValue { source })?,
            );
        }
        let mut chunks = given.chunks.clone();
        if let Some(text) = &given.text {
            chunks.push(text.as_bytes().to_vec());
        }
        Ok(Self {
            status: given.status,
            headers,
            chunks,
            seen: OnceLock::new(),
        })
    }
}

impl FileTransport for Fixture {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        drop(self.seen.set(request));
        let mut body = Vec::new();
        for chunk in &self.chunks {
            body.extend_from_slice(chunk);
        }
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body,
        }))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        drop(self.seen.set(request));
        ready(Ok(StreamReply {
            status: self.status,
            headers: self.headers.clone(),
            body: stream::iter(self.chunks.clone().into_iter().map(Ok)).boxed(),
        }))
    }
}

pub(super) fn request_view(fixture: &Fixture, extra_headers: &IndexMap<String, String>) -> Value {
    let Some(request) = fixture.seen.get() else {
        return Value::Null;
    };
    let header = |name: &str| {
        request
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let extra: IndexMap<&str, Option<String>> = extra_headers
        .keys()
        .map(|name| (name.as_str(), header(name)))
        .collect();
    json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "accept": header("accept"),
        "authorization": header("authorization"),
        "organization": header("openai-organization"),
        "project": header("openai-project"),
        "extra_headers": extra,
        "body": body_json(request),
    })
}

fn body_json(request: &OutboundRequest) -> Value {
    match &request.body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Value::Null
        }
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

pub(super) fn logging_view(recorder: &Recorder) -> Value {
    let Some(record) = recorder.records.first() else {
        return Value::Null;
    };
    json!({
        "input": record.input.as_str(),
        "api_base": record.api_base.as_str(),
        "body": &record.body,
    })
}
