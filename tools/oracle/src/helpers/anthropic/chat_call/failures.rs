use std::convert::Infallible;

use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::ClientKind;
use betterlite_anthropic::headers::HeadersError;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::Input;
use super::fixture::FixtureFailure;
use super::scenarios::{Run, client_kind, execute};

// the exception LiteLLM raises for the same call, named the way Python names it
pub(super) struct FailureShape {
    class: &'static str,
    status: Option<u16>,
    message: String,
}

impl FailureShape {
    pub(super) fn of(error: &ChatCallError<FixtureFailure, Infallible>) -> Self {
        let class = match error {
            ChatCallError::Anthropic { .. } => "AnthropicError",
            ChatCallError::Headers(HeadersError::Authentication { .. }) => "AuthenticationError",
            ChatCallError::Headers(_) => "HeadersError",
            ChatCallError::Prepare { .. } | ChatCallError::Filter { .. } => "ValueError",
            ChatCallError::PreCall(_) => "PreCallError",
            ChatCallError::Response(ResponseError::Anthropic { .. }) => "AnthropicError",
            ChatCallError::Response(_) | ChatCallError::Parameter(_) => "ValueError",
            ChatCallError::Open(_) => "AnthropicError",
            ChatCallError::Endpoint { .. }
            | ChatCallError::HeaderName { .. }
            | ChatCallError::HeaderValue { .. } => "ValueError",
        };
        Self {
            class,
            status: error.status_code(),
            message: error.to_string(),
        }
    }
}

// the case compares the exception LiteLLM raises, so the answer is that error, not a value
fn failure_answer(run: &Run) -> HelperResult {
    match &run.failure {
        Some(failure) => Err(Box::new(HelperError {
            class: failure.class.to_owned(),
            status: failure.status,
            error_type: None,
            param: None,
            code: None,
            message: failure.message.clone(),
        })),
        None => Err("the call was expected to fail and did not").or_value_error(),
    }
}

pub(super) fn pre_stream_error(input: &Input) -> HelperResult {
    let acompletion = input.acompletion;
    let run = execute(input, acompletion, client_kind(acompletion))?;
    failure_answer(&run)
}

pub(super) fn native_error_body(input: &Input) -> HelperResult {
    let acompletion = input.acompletion;
    let run = execute(input, acompletion, client_kind(acompletion))?;
    failure_answer(&run)
}

pub(super) fn chat_failure_projection(input: &Input) -> HelperResult {
    let asynchronous = execute(input, true, ClientKind::Async)?;
    if asynchronous.failure.is_none() {
        return Err("the call was expected to fail and did not").or_value_error();
    }
    failure_answer(&asynchronous)
}
