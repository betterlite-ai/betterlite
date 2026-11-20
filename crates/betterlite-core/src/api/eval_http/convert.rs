use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::openai_base_config::Headers;

use crate::api::vector_store_http_write::error::{
    FailureFacts, HeaderMap, display_facts, handle_error, status_facts, timeout_facts,
};
use crate::api::vector_store_http_write::service::ClientKind;

use super::error::EvalHttpError;
use super::exchange::{EvalConfig, EvalMethod};

// The method, the deadline the send ran with and how long it took, which the timeout message repeats
#[derive(Debug, Clone, Copy)]
pub(super) struct SendFacts {
    pub(super) method: EvalMethod,
    pub(super) deadline: Duration,
    pub(super) elapsed: Duration,
}

// B2402 keeps every header as a JSON value, and the evals error class carries the text ones, which is every header it builds
fn text_headers(headers: HeaderMap) -> Headers {
    headers
        .into_iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name, text)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

// The shared conversion: B2402's handle_error picks the status, text and headers, and the provider config names the error class
pub(super) fn failed(config: EvalConfig<'_>, facts: &FailureFacts) -> EvalHttpError {
    let converted = handle_error(facts, None);
    let headers = text_headers(converted.headers);
    let mut error = config.get_error_class(&converted.message, converted.status_code, &headers);
    if let EvalsError::Provider { provider } = &mut error {
        provider.status_code_is_synthesized = converted.status_code_is_synthesized;
    }
    EvalHttpError::from(error)
}

// The async post catches its timeout and raises litellm.Timeout; get and the async delete re-raise it, and every other failure, as it is
pub(super) fn send_failed(
    config: EvalConfig<'_>,
    facts: &SendFacts,
    error: &TransportError,
) -> EvalHttpError {
    let timed_out = matches!(
        error,
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_)
    );
    let raised = if facts.method == EvalMethod::Post && timed_out {
        timeout_facts(
            ClientKind::Asynchronous,
            Some(facts.deadline),
            facts.elapsed,
        )
    } else {
        display_facts(error.to_string())
    };
    failed(config, &raised)
}

// post and delete call raise_for_status, which raises for every status outside 2xx, so a 3xx the client did not follow is converted too
pub(super) fn check_status(
    config: EvalConfig<'_>,
    reply: UpstreamResponse,
) -> Result<UpstreamResponse, EvalHttpError> {
    if (200..=299).contains(&reply.status) {
        return Ok(reply);
    }
    Err(failed(config, &status_facts(&reply)))
}
