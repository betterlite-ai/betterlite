use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;

use super::super::error::{FailureFacts, display_facts, timeout_facts};
use super::{ClientKind, Operation};

// What the timeout message needs from the send that just failed; operation only picks the async delete exemption
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sent {
    pub(crate) mode: ClientKind,
    pub(crate) operation: Operation,
    pub(crate) timeout: Option<Duration>,
    pub(crate) elapsed: Duration,
}

pub(super) fn http_response(reply: &UpstreamResponse) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(reply.body.clone());
    *response.status_mut() =
        http::StatusCode::from_u16(reply.status).unwrap_or(http::StatusCode::OK);
    *response.headers_mut() = reply.headers.clone();
    response
}

pub(crate) fn transport_facts(error: &TransportError, sent: &Sent) -> FailureFacts {
    if caught_timeout(error, sent.mode, sent.operation) {
        return timeout_facts(sent.mode, sent.timeout, sent.elapsed);
    }
    display_facts(error.to_string())
}

// Only the update handlers and the sync delete handler catch httpx.TimeoutException; the async delete does not
fn caught_timeout(error: &TransportError, mode: ClientKind, operation: Operation) -> bool {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    timed_out && !(mode == ClientKind::Asynchronous && operation == Operation::Delete)
}
