use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_redact::secret::Redactor;

const UNSUPPORTED_PREFIX: &str = "litellm.UnsupportedParamsError: ";

// LiteLLM's exception_type returns its own exception classes unchanged, so the class is carried, not re-derived
#[derive(Debug, Clone)]
pub enum ImageGenerationException {
    Mapped { class: ErrorClass, message: String },
    UnknownProvider { message: String },
}

impl ImageGenerationException {
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Mapped { message, .. } | Self::UnknownProvider { message } => message,
        }
    }
}

pub(crate) fn raise(redactor: &Redactor, failure: RawException) -> ImageGenerationException {
    let raw = failure
        .message
        .clone()
        .unwrap_or_else(|| failure.error_str.clone());
    let text = redactor.redact_string(&raw);
    ImageGenerationException::Mapped {
        class: dispatch(&DispatchInput::Raw(Box::new(failure)), redactor),
        message: text,
    }
}

// str(e) of a plain Exception or ValueError: no message attribute overrides it and no status is carried
pub(crate) fn value_failure(
    message: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> RawException {
    RawException {
        model: model.unwrap_or_default().to_owned(),
        custom_llm_provider: provider.unwrap_or_default().to_owned(),
        error_str: message.to_owned(),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    }
}

// A provider exception: str(e) and the attributes exception_type() reads, with the status the provider answered
#[must_use]
pub fn provider_failure(
    status_code: u16,
    message: &str,
    model: &str,
    provider: &str,
    exception_type: &str,
) -> RawException {
    RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.to_owned(),
        status_code: Some(status_code),
        status_code_is_synthesized: false,
        message: Some(message.to_owned()),
        body: None,
        exception_type: exception_type.to_owned(),
    }
}

// get_llm_provider wraps every failure of its try block in a BadRequestError with the original model appended
#[must_use]
pub fn resolution_failure(message: &str, model: Option<&str>) -> RawException {
    let text = format!(
        "GetLLMProvider Exception - {message}\n\noriginal model: {}",
        model.unwrap_or("None")
    );
    let mut failure = value_failure(&text, model, None);
    failure.status_code = Some(400);
    failure.message = Some(text);
    failure.exception_type = String::from("BadRequestError");
    failure
}

#[must_use]
pub fn unsupported_params(message: &str) -> ImageGenerationException {
    let text = format!("{UNSUPPORTED_PREFIX}{message}");
    ImageGenerationException::Mapped {
        class: ErrorClass::UnsupportedParams(Box::new(context(text.clone()))),
        message: text,
    }
}

#[must_use]
pub fn unknown_provider(model: Option<&str>, provider: &str) -> ImageGenerationException {
    let message = format!(
        "Unmapped LLM provider for this endpoint. You passed model={}, custom_llm_provider={provider}. Check supported provider and route: https://docs.litellm.ai/docs/providers",
        model.unwrap_or("None")
    );
    ImageGenerationException::UnknownProvider { message }
}

const fn context(message: String) -> ErrorContext {
    ErrorContext {
        message,
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}
