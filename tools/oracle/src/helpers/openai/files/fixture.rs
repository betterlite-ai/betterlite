use std::future::{Future, ready};
use std::sync::OnceLock;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    status: u16,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    json: Option<Value>,
    #[serde(default)]
    chunks: Vec<Vec<u8>>,
    #[serde(default)]
    fail_after: bool,
}

// One recorded upstream answer; the request the production code sent is kept for the comparison
pub(super) struct Fixture {
    status: u16,
    headers: HeaderMap,
    chunks: Vec<Vec<u8>>,
    fail_after: bool,
    seen: OnceLock<OutboundRequest>,
}

impl Fixture {
    // Without a reply the call is not expected to reach the transport
    pub(super) fn new(given: Option<&Reply>) -> Result<Self, String> {
        let Some(reply) = given else {
            return Ok(Self {
                status: 599,
                headers: HeaderMap::new(),
                chunks: Vec::new(),
                fail_after: false,
                seen: OnceLock::new(),
            });
        };
        let mut headers = HeaderMap::new();
        for (name, value) in &reply.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).map_err(|error| error.to_string())?,
                HeaderValue::from_str(value).map_err(|error| error.to_string())?,
            );
        }
        let chunks = reply.json.as_ref().map_or_else(
            || reply.chunks.clone(),
            |body| vec![body.to_string().into_bytes()],
        );
        Ok(Self {
            status: reply.status,
            headers,
            chunks,
            fail_after: reply.fail_after,
            seen: OnceLock::new(),
        })
    }

    // The reply headers the comparison looks at, by lower-case name
    pub(super) fn watched(&self, flat: &IndexMap<String, String>) -> Value {
        let picked: IndexMap<&str, Option<&String>> = self
            .headers
            .keys()
            .map(|name| (name.as_str(), flat.get(name.as_str())))
            .collect();
        json!(picked)
    }

    fn buffered_reply(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.chunks.concat(),
        }
    }

    fn stream_reply(&self) -> StreamReply {
        let mut items: Vec<Result<Vec<u8>, HttpError>> =
            self.chunks.iter().cloned().map(Ok).collect();
        if self.fail_after {
            items.push(Err(HttpError::Timeout));
        }
        StreamReply {
            status: self.status,
            headers: self.headers.clone(),
            body: stream::iter(items).boxed(),
        }
    }

    pub(super) fn request_view(&self, extra_headers: &IndexMap<String, String>) -> Value {
        let Some(request) = self.seen.get() else {
            return Value::Null;
        };
        let header = |name: &str| {
            let value = request.headers.get(name)?;
            value.to_str().ok()
        };
        let extra: IndexMap<&str, Option<&str>> = extra_headers
            .keys()
            .map(|name| (name.as_str(), header(name)))
            .collect();
        let parts = match &request.body {
            RequestBody::Multipart { parts, .. } => Some(
                parts
                    .iter()
                    .map(|part| {
                        json!({
                            "name": part.name,
                            "filename": part.file_name,
                            "content_type": part.content_type,
                            "text": std::str::from_utf8(&part.data).unwrap_or_default(),
                        })
                    })
                    .collect::<Vec<Value>>(),
            ),
            RequestBody::Empty | RequestBody::Json(_) | RequestBody::Bytes { .. } => None,
        };
        json!({
            "method": request.method.as_str(),
            "path": request.url.path(),
            "query": request.query,
            "accept": header("accept"),
            "authorization": header("authorization"),
            "organization": header("openai-organization"),
            "extra_headers": extra,
            "timeout": request.deadline.as_secs(),
            "parts": parts,
        })
    }
}

impl FileTransport for &Fixture {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        drop(self.seen.set(request));
        ready(Ok(self.buffered_reply()))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        drop(self.seen.set(request));
        ready(Ok(self.stream_reply()))
    }
}
