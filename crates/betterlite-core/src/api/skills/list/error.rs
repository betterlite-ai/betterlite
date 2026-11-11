use betterlite_anthropic::skills::error::SkillsError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::skills::get_delete::error::SkillsCallError;
use crate::api::skills::get_delete::handler::SkillsHandlerError;

use super::call::ListSkillsCall;

#[derive(Debug, Error)]
pub enum ListSkillsError {
    #[error("LIST skills is not supported for {provider}")]
    Unsupported { provider: String },
    #[error("the {name} field does not have the declared type")]
    FieldType { name: String },
    #[error(transparent)]
    Call(#[from] SkillsCallError),
}

impl From<SkillsError> for ListSkillsError {
    fn from(source: SkillsError) -> Self {
        Self::Call(SkillsCallError::from(source))
    }
}

impl From<LoggingStateError> for ListSkillsError {
    fn from(source: LoggingStateError) -> Self {
        Self::Call(SkillsCallError::from(source))
    }
}

impl From<SkillsHandlerError> for ListSkillsError {
    fn from(source: SkillsHandlerError) -> Self {
        Self::Call(SkillsCallError::from(source))
    }
}

#[derive(Debug, Error)]
#[error("{error}")]
pub struct ListSkillsFailureError {
    #[source]
    pub error: ListSkillsError,
    pub class: ErrorClass,
}

// LiteLLM's exception_type(model=None, custom_llm_provider=..., original_exception=...)
#[must_use]
pub fn failure(
    error: ListSkillsError,
    call: &ListSkillsCall<'_>,
    redactor: &Redactor,
) -> ListSkillsFailureError {
    let provider = call.params.provider();
    let class = match &error {
        // docs/compatibility.md rejects a value outside the declared type with a 400 naming the field, not a provider error
        ListSkillsError::FieldType { name } => {
            ErrorClass::BadRequest(Box::new(ResponseDetails::with_status(
                ErrorContext {
                    message: error.to_string(),
                    model: None,
                    provider: Some(provider.to_owned()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: Some(name.clone()),
                    body_code: None,
                },
                None,
            )))
        }
        ListSkillsError::Unsupported { .. } | ListSkillsError::Call(_) => dispatch(
            &DispatchInput::Raw(Box::new(raw_exception(&error, provider))),
            redactor,
        ),
    };
    ListSkillsFailureError { error, class }
}

fn raw_exception(error: &ListSkillsError, provider: &str) -> RawException {
    let ListSkillsError::Call(SkillsCallError::Handler(SkillsHandlerError::Provider {
        status_code,
        message,
        status_code_is_synthesized,
        body_code,
        body_message,
    })) = error
    else {
        let text = error.to_string();
        return RawException {
            model: String::new(),
            custom_llm_provider: provider.to_owned(),
            error_str: text.clone(),
            status_code: None,
            status_code_is_synthesized: false,
            message: Some(text),
            body: None,
            exception_type: "ValueError".to_owned(),
        };
    };
    RawException {
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
    }
}
