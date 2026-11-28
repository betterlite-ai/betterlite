use std::convert::Infallible;

use betterlite_anthropic::chat_call::CallStyle;
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::headers::HeadersError;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_http::client_kind::ClientKind;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::Input;
use super::fixture::FixtureFailure;
use super::scenarios::{Run, client_kind, execute};

pub(super) struct FailureShape {
    class: &'static str,
    status: Option<u16>,
    message: String,
}

impl FailureShape {
    pub(super) fn of(error: &ChatCallError<FixtureFailure, Infallible>) -> Self {
        let class = match error {
            ChatCallError::Anthropic { .. }
            | ChatCallError::Response(ResponseError::Anthropic { .. })
            | ChatCallError::Open(_) => "AnthropicError",
            ChatCallError::Headers(HeadersError::Authentication { .. }) => "AuthenticationError",
            ChatCallError::Headers(_) => "HeadersError",
            ChatCallError::Prepare { .. }
            | ChatCallError::Filter { .. }
            | ChatCallError::Response(_)
            | ChatCallError::Parameter(_)
            | ChatCallError::Endpoint(_)
            | ChatCallError::HeaderName(_)
            | ChatCallError::HeaderValue(_) => "ValueError",
            ChatCallError::PreCall(_) => "PreCallError",
        };
        Self {
            class,
            status: error.status_code(),
            message: error.to_string(),
        }
    }
}

fn failure_answer(run: &Run) -> HelperResult {
    run.failure.as_ref().map_or_else(
        || Err("the call was expected to fail and did not").or_value_error(),
        |failure| {
            Err(Box::new(HelperError {
                class: failure.class.to_owned(),
                status: failure.status,
                error_type: None,
                param: None,
                code: None,
                message: failure.message.clone(),
            }))
        },
    )
}

// both cases compare the exception LiteLLM raises, so the answer is that error and not a value
pub(super) fn failed_call(input: &Input) -> HelperResult {
    let style = if input.acompletion {
        CallStyle::Async
    } else {
        CallStyle::Sync
    };
    let run = execute(input, style, client_kind(input.acompletion))?;
    failure_answer(&run)
}

pub(super) fn chat_failure_projection(input: &Input) -> HelperResult {
    let asynchronous = execute(input, CallStyle::Async, ClientKind::Asynchronous)?;
    if asynchronous.failure.is_none() {
        return Err("the call was expected to fail and did not").or_value_error();
    }
    failure_answer(&asynchronous)
}
