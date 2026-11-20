use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;

use crate::api::vector_store_http_write::error::{FailureFacts, display_facts, timeout_facts};
use crate::api::vector_store_http_write::service::ClientKind;

// What the timeout message needs: the caller's timeout, the one httpx used and how long the send took
pub(super) struct Attempt {
    pub(super) mode: ClientKind,
    pub(super) timeout: Option<Duration>,
    pub(super) effective: Duration,
    pub(super) elapsed: Duration,
}

// A timeout carries 408 and its message; anything else the transport raises is synthesized into a 500
pub(super) fn transport_facts(error: &TransportError, attempt: &Attempt) -> FailureFacts {
    if !timed_out(error) {
        return display_facts(error.to_string());
    }
    // post catches httpx.TimeoutException in both forms: sync echoes the timeout it was given, async the deadline it ran with
    let timeout = match attempt.mode {
        ClientKind::Sync => attempt.timeout,
        ClientKind::Asynchronous => Some(attempt.timeout.unwrap_or(attempt.effective)),
    };
    timeout_facts(attempt.mode, timeout, attempt.elapsed)
}

const fn timed_out(error: &TransportError) -> bool {
    matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    )
}
