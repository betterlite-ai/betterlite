use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use http::header::{CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue};

use super::super::input::Reply;

pub(crate) type Log = Arc<Mutex<Vec<OutboundRequest>>>;

pub(crate) struct Recorded {
    reply: UpstreamResponse,
    calls: Log,
}

impl Recorded {
    pub(crate) fn new(reply: Option<&Reply>, calls: Log) -> Self {
        Self {
            reply: response(reply),
            calls,
        }
    }
}

impl FileTransport for Recorded {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        ready(Ok(self.reply.clone()))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        ready(Err(TransportError::HeadersTooLarge { limit: 0 }))
    }
}

// httpx adds a content-length header for a non-empty body, and keeps every declared header
fn response(reply: Option<&Reply>) -> UpstreamResponse {
    let Some(reply) = reply else {
        return UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
        };
    };
    let mut headers = HeaderMap::new();
    for (name, value) in &reply.headers {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).expect("the reply header name is valid"),
            HeaderValue::from_str(value).expect("the reply header value is valid"),
        );
    }
    if !reply.body.is_empty() && !headers.contains_key(CONTENT_LENGTH) {
        headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&reply.body.len().to_string())
                .expect("the reply body length is a header value"),
        );
    }
    UpstreamResponse {
        status: reply.status,
        headers,
        body: reply.body.clone().into_bytes(),
    }
}
