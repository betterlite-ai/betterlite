use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, FailureFacts, display_facts, handle_error, timeout_facts,
};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

use super::super::config::FileWriteConfig;

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(super) struct Sent {
    pub(super) mode: ClientKind,
    pub(super) operation: Operation,
    pub(super) given: Option<Duration>,
    pub(super) timeout: Duration,
    pub(super) elapsed: Duration,
}

pub(super) fn http_response(reply: &UpstreamResponse) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(reply.body.clone());
    *response.status_mut() =
        http::StatusCode::from_u16(reply.status).unwrap_or(http::StatusCode::OK);
    *response.headers_mut() = reply.headers.clone();
    response
}

// _handle_error's answer, asked of the provider's own error class the way LiteLLM asks it, with the synthesized status kept
pub(super) fn provider_error(
    config: &dyn FileWriteConfig,
    facts: &FailureFacts,
) -> BaseLlmException {
    let mapped = handle_error(facts, None);
    let mut failure = config.error_class(&mapped.message, mapped.status_code, mapped.headers);
    failure.status_code_is_synthesized = mapped.status_code_is_synthesized;
    failure
}

pub(super) fn transport_facts(error: &TransportError, sent: &Sent) -> FailureFacts {
    if caught_timeout(error, sent.mode, sent.operation) {
        // the file handlers hand the timeout to both clients, so the message shows it exactly as the client did
        let timeout = match sent.mode {
            ClientKind::Sync => sent.given,
            ClientKind::Asynchronous => Some(sent.timeout),
        };
        return timeout_facts(sent.mode, timeout, sent.elapsed);
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
