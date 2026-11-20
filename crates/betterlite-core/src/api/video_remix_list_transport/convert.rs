use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;

use crate::api::vector_store_http_write::error::{
    FailureFacts, display_facts, handle_error, status_facts, timeout_facts,
};
use crate::api::vector_store_http_write::service::ClientKind;

use super::config::VideoRemixListConfig;
use super::error::{Operation, VideoRemixListTransportError};

// The handler mode the send ran in, the timeout it was given and how long the send took
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SendFacts {
    pub(super) operation: Operation,
    pub(super) mode: ClientKind,
    pub(super) timeout: Option<Duration>,
    pub(super) elapsed: Duration,
}

// The shared conversion: B2402's handle_error picks the status, text and headers, and the provider config names the error class
pub(super) fn failure(
    config: &dyn VideoRemixListConfig,
    facts: &FailureFacts,
) -> VideoRemixListTransportError {
    let converted = handle_error(facts, None);
    let mut error =
        config.get_error_class(&converted.message, converted.status_code, converted.headers);
    error.status_code_is_synthesized = converted.status_code_is_synthesized;
    VideoRemixListTransportError::Provider(Box::new(error))
}

// Only the remix handlers catch httpx.TimeoutException and raise litellm.Timeout; a list GET lets it escape as a raw exception
pub(super) fn transport_facts(facts: &SendFacts, error: &TransportError) -> FailureFacts {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    if timed_out && facts.operation == Operation::Remix {
        return timeout_facts(facts.mode, facts.timeout, facts.elapsed);
    }
    display_facts(error.to_string())
}

// Both handlers call raise_for_status on their send, so a 4xx or 5xx answer reaches error conversion before the response transform
pub(super) fn check_status(
    config: &dyn VideoRemixListConfig,
    reply: UpstreamResponse,
) -> Result<UpstreamResponse, VideoRemixListTransportError> {
    if reply.status < 400 {
        return Ok(reply);
    }
    Err(failure(config, &status_facts(&reply)))
}
