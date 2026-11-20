use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::api::vector_store_http_write::error::{FailureFacts, HeaderMap, display_facts};

use super::ClientKind;

// What the timeout message needs from the send that just failed
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sent {
    pub(crate) mode: ClientKind,
    pub(crate) requested: Option<Duration>,
    pub(crate) effective: Duration,
    pub(crate) elapsed: Duration,
}

// A connect or read timeout becomes litellm.Timeout with status 408 and no headers; other failures synthesize a 500
pub(crate) fn transport_facts(error: &TransportError, sent: &Sent) -> FailureFacts {
    if matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    ) {
        return FailureFacts {
            response_status: None,
            status: Some(Json::Number(Number::from(408_u16))),
            text: Some(timeout_message(sent)),
            headers: Some(HeaderMap::new()),
            response: None,
            display: String::new(),
        };
    }
    display_facts(error.to_string())
}

// The sync handler prints the caller's timeout and the async one the timeout it used, as Python's str would
pub(crate) fn timeout_message(sent: &Sent) -> String {
    match sent.mode {
        ClientKind::Sync => {
            let passed = sent.requested.map_or_else(
                || String::from("None"),
                |timeout| format!("{:?}", timeout.as_secs_f64()),
            );
            format!("litellm.Timeout: Connection timed out after {passed} seconds.")
        }
        ClientKind::Asynchronous => {
            // Python rounds to three decimals; formatting at that precision and trimming zeros gives the same text
            let rounded = format!("{:.3}", sent.elapsed.as_secs_f64());
            let trimmed = rounded.trim_end_matches('0');
            let taken = if trimmed.ends_with('.') {
                format!("{trimmed}0")
            } else {
                trimmed.to_owned()
            };
            format!(
                "litellm.Timeout: Connection timed out. Timeout passed={:?}, time taken={taken} seconds",
                sent.effective.as_secs_f64(),
            )
        }
    }
}
