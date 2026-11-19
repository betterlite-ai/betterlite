use std::convert::Infallible;

use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_redact::error::RedactionError;
use betterlite_streaming::passthrough_reconstruction::error::ReconstructionError;
use thiserror::Error;

// The Anthropic chunk conversion fails either on a shape the event converter rejects or on the serde hand-off.
#[derive(Debug, Error)]
pub enum ConvertFailure {
    #[error(transparent)]
    Event(#[from] StreamEventError<Infallible>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum ProcessStreamError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Undeliverable(#[from] UndeliverableStreamRewrite),
    #[error("the guardrail redactor could not be built")]
    Redactor {
        #[from]
        source: RedactionError,
    },
    #[error("the buffered stream could not be assembled")]
    Assembly {
        #[source]
        source: Box<ReconstructionError<ConvertFailure>>,
    },
    #[error("the guardrail data could not be read as JSON")]
    Convert(#[from] serde_json::Error),
}

// Both paths hand a ModifyResponse its fallback only when the guardrail left original_response empty.
pub(super) fn keep_original_response(
    failure: GuardrailFailure,
    fallback: Option<GuardrailValue>,
) -> GuardrailFailure {
    match failure {
        GuardrailFailure::ModifyResponse(mut response) => {
            if response.original_response.is_none() {
                response.original_response = fallback;
            }
            GuardrailFailure::ModifyResponse(response)
        }
        other @ GuardrailFailure::GuardrailRaised { .. }
        | other @ GuardrailFailure::BlockedPii { .. }
        | other @ GuardrailFailure::SensitiveDataRoute(_)
        | other @ GuardrailFailure::Http { .. }
        | other @ GuardrailFailure::HttpObject { .. }
        | other @ GuardrailFailure::Other(_) => other,
    }
}
