use std::time::TryFromFloatSecsError;

use betterlite_anthropic::skills::error::SkillsError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use super::call::SkillCall;
use super::handler::SkillsHandlerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillAction {
    Get,
    Delete,
}

impl SkillAction {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Error)]
pub enum SkillsCallError {
    #[error("{} skill is not supported for {provider}", action.label())]
    Unsupported {
        action: SkillAction,
        provider: String,
    },
    #[error(transparent)]
    Skills(#[from] SkillsError),
    #[error(transparent)]
    Logging(#[from] LoggingStateError),
    #[error(transparent)]
    Handler(#[from] SkillsHandlerError),
    #[error("the request timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct SkillsFailureError {
    #[source]
    pub error: SkillsCallError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(
    error: SkillsCallError,
    call: &SkillCall<'_>,
    redactor: &Redactor,
) -> SkillsFailureError {
    let provider = call.params.provider();
    let class = dispatch(
        &DispatchInput::Raw(Box::new(raw_exception(&error, provider))),
        redactor,
    );
    SkillsFailureError { error, class }
}

fn raw_exception(error: &SkillsCallError, provider: &str) -> RawException {
    match error {
        SkillsCallError::Handler(SkillsHandlerError::Provider {
            status_code,
            message,
            status_code_is_synthesized,
            body_code,
            body_message,
        }) => RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: message.clone(),
            status_code: Some(*status_code),
            status_code_is_synthesized: *status_code_is_synthesized,
            message: Some(message.clone()),
            body: Some(RawExceptionBody {
                kind: None,
                param: None,
                code: body_code.clone(),
                message: body_message.clone(),
                error: None,
            }),
            exception_type: String::new(),
        },
        SkillsCallError::Unsupported { .. }
        | SkillsCallError::Skills(_)
        | SkillsCallError::Logging(_)
        | SkillsCallError::Handler(SkillsHandlerError::Skills(_))
        | SkillsCallError::Timeout { .. } => RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: error.to_string(),
            status_code: None,
            status_code_is_synthesized: false,
            message: Some(error.to_string()),
            body: None,
            exception_type: "ValueError".to_owned(),
        },
    }
}
