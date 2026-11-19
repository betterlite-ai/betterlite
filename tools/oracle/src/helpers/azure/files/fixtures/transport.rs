use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_oracle_helper_api::error::HelperError;
use http::header::{CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue};

use super::super::input::Reply;

pub(crate) type Log = Arc<Mutex<Vec<OutboundRequest>>>;

pub(crate) struct Recorded {
    reply: UpstreamResponse,
    calls: Log,
}

impl Recorded {
    pub(crate) fn new(reply: Option<&Reply>, calls: Log) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            reply: response(reply)?,
            calls,
        })
    }
}

impl FileTransport for Recorded {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(answer(&self.calls, request, Ok(self.reply.clone())))
    }

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        ready(answer(
            &self.calls,
            request,
            Err(TransportError::HeadersTooLarge { limit: 0 }),
        ))
    }
}

// every request the SDK sends is kept, so a case can report the wire call it made
fn answer<T>(calls: &Log, request: OutboundRequest, reply: T) -> T {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
    reply
}

// httpx adds a content-length header for a non-empty body, and keeps every declared header
fn response(reply: Option<&Reply>) -> Result<UpstreamResponse, Box<HelperError>> {
    let Some(provided) = reply else {
        return Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
        });
    };
    let mut headers = HeaderMap::new();
    for (name, value) in &provided.headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| invalid(source.to_string()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| invalid(source.to_string()))?;
        headers.append(header_name, header_value);
    }
    let length = HeaderValue::from_str(&provided.body.len().to_string()).ok();
    if let Some(header_length) = length.filter(|_| !provided.body.is_empty()) {
        headers.entry(CONTENT_LENGTH).or_insert(header_length);
    }
    Ok(UpstreamResponse {
        status: provided.status,
        headers,
        body: provided.body.clone().into_bytes(),
    })
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
