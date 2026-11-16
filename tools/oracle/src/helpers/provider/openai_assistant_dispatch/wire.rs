use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::stream::{RunChunks, RunStreamTransport};
use futures_util::stream;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue};
use serde_json::{Value, json};

use super::input::Reply;

// The canned upstream the composition talks to: it records every request and answers from the case's replies.
pub(crate) struct Wire {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Value>>,
}

impl Wire {
    pub(crate) fn replying(replies: &[Reply]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            seen: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn seen(&self) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, request: &OutboundRequest) -> Reply {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Value::Null
            }
        };
        let content_type = header(&request.headers, "content-type")
            .or_else(|| (!body.is_null()).then(|| String::from("application/json")));
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "method": request.method.as_str(),
                "url": url.as_str(),
                "authorization": header(&request.headers, "authorization"),
                "openai-organization": header(&request.headers, "openai-organization"),
                "openai-beta": header(&request.headers, "openai-beta"),
                "content-type": content_type,
                "body": body,
            }));
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn answer_bytes(reply: &Reply) -> Vec<u8> {
    match (&reply.text, &reply.body) {
        (Some(text), _present) => text.clone().into_bytes(),
        (None, Some(body)) => serde_json::to_vec(body).unwrap_or_else(|_error| b"null".to_vec()),
        (None, None) => b"null".to_vec(),
    }
}

fn answer_headers(reply: &Reply) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(text) = reply.content_type.as_deref()
        && let Ok(value) = HeaderValue::try_from(text)
    {
        headers.insert(CONTENT_TYPE, value);
    }
    headers
}

impl AssistantsTransport for Wire {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let reply = self.record(&request);
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: answer_headers(&reply),
            body: answer_bytes(&reply),
        }))
    }
}

// The proxied client reads an error status before any frame, so a refused open fails the open itself.
impl RunStreamTransport for Wire {
    fn open(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        let reply = self.record(&request);
        let body = answer_bytes(&reply);
        if reply.status >= 400 {
            return ready(Err(ThreadRunsError::Status(reply.status, body)));
        }
        let frame: Result<Vec<u8>, ThreadRunsError> = Ok(body);
        let chunks: RunChunks = Box::pin(stream::iter([frame]));
        ready(Ok(chunks))
    }
}
