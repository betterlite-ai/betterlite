use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;

use crate::api::vector_store_http_write::error::{FailureFacts, display_facts, timeout_facts};
use crate::api::vector_store_http_write::service::ClientKind;

use super::Operation;

// What a failed send leaves for the error record: the mode and timeout the message names, and how long it ran
#[derive(Debug, Clone, Copy)]
pub(super) struct Attempt {
    pub(super) operation: Operation,
    pub(super) mode: ClientKind,
    pub(super) timeout: Option<Duration>,
    pub(super) elapsed: Duration,
}

// The create handlers wrap their send in httpx's timeout translation; the list get lets a timeout fall through as a synthesized 500
pub(super) fn send_facts(error: &TransportError, attempt: &Attempt) -> FailureFacts {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    if attempt.operation == Operation::Create && timed_out {
        return timeout_facts(attempt.mode, attempt.timeout, attempt.elapsed);
    }
    display_facts(error.to_string())
}
