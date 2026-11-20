use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_redact::secret::Redactor;

use super::call::StreamAbort;
use super::framing::ChunkFraming;
use super::state::Guard;
use crate::sse_data_generator::types::{
    FailureKind, FailureParam, FailureType, StreamFailure, StreamHooks,
};
use crate::sse_error_frames::{ErrorFields, SseFailure, sse_error_payload};

#[must_use]
pub fn failure_exception(failure: &StreamFailure, redactor: Option<&Redactor>) -> ProxyException {
    let message = redactor.map_or_else(
        || failure.message.clone(),
        |active| active.redact_internal_details(&failure.message),
    );
    let (status, body) = sse_error_payload(&shaped_failure(failure, message));
    ProxyException::new(ProxyExceptionInit {
        message: body_text(&body, "message"),
        error_type: body_text(&body, "type"),
        param: match body.get("param") {
            Some(JsonValue::String(text)) => Some(text.clone()),
            Some(
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_),
            )
            | None => None,
        },
        code: ExceptionCode::Status(i64::from(status)),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn shaped_failure(failure: &StreamFailure, message: String) -> SseFailure {
    SseFailure {
        status_code: failure.status_code,
        code: None,
        error_type: match &failure.error_type {
            FailureType::Text(text) => Some(text.clone()),
            FailureType::Missing | FailureType::Null => None,
        },
        param: match &failure.param {
            FailureParam::Text(text) => Some(text.clone()),
            FailureParam::Missing | FailureParam::Null => None,
        },
        detail: Some(Box::new(JsonValue::String(message))),
        provider_specific_fields: None,
    }
}

// sse_error_payload writes the message and type as strings and leaves param null when the failure carried none
fn body_text(body: &ErrorFields, key: &str) -> String {
    match body.get(key) {
        Some(JsonValue::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

pub(super) async fn finish_failure<Key, H, F>(
    hooks: &H,
    framing: &mut F,
    guard: &mut Guard<Key>,
    failure: StreamFailure,
    redactor: Option<&Redactor>,
) -> Result<F::Out, StreamAbort>
where
    Key: Send + Sync,
    H: StreamHooks<Key>,
    F: ChunkFraming,
{
    tracing::error!(
        "litellm.proxy.proxy_server.async_data_generator(): Exception occurred - {failure}"
    );
    let replacement = match guard.outcome.as_ref() {
        Some(outcome) => hooks
            .failure_hook(&outcome.key, &failure, &outcome.request_data)
            .await
            .err(),
        None => None,
    };
    let effective = replacement.unwrap_or(failure);
    if effective.kind == FailureKind::Http {
        guard.sealed = true;
        return Err(StreamAbort {
            status: effective.status_code.unwrap_or(500),
            detail: effective.message,
        });
    }
    let exception = failure_exception(&effective, redactor);
    guard.mark_completed();
    framing.frame_failure(&exception)
}
