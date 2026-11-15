use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_redact::secret::Redactor;

use crate::text_completion_request::CompletionRequestError;
use crate::text_completion_transport::error::{
    ErrorStatus, TextCompletionError, TextCompletionFailure,
};

use super::catalog::CompletionRoute;

// The status, type, param and message the route answers with; a null type is how LiteLLM serializes it
#[derive(Debug)]
pub struct CompletionFailure {
    pub status: u16,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub message: String,
}

impl CompletionFailure {
    const fn new(status: u16, error_type: Option<String>, message: String) -> Self {
        Self {
            status,
            error_type,
            param: None,
            message,
        }
    }

    pub(super) fn internal(message: String) -> Self {
        Self::new(500, Some(String::from("internal_server_error")), message)
    }

    // main.py's own "Unmapped prompt format" Exception reaches the client as an APIConnectionError, whose type is null
    pub(super) const fn unmapped(message: String) -> Self {
        Self::new(500, None, message)
    }

    pub(super) fn route(route: &CompletionRoute) -> Self {
        match route {
            CompletionRoute::Text { .. } => Self::internal(String::from(
                "betterlite does not serve this completion deployment yet",
            )),
            CompletionRoute::Chat { .. } => Self::new(
                500,
                Some(String::from("internal_server_error")),
                String::from("betterlite does not serve chat-model text completions yet"),
            ),
            CompletionRoute::Unsupported { provider } => Self::new(
                501,
                Some(String::from("internal_server_error")),
                format!("betterlite does not serve text completions for {provider}"),
            ),
        }
    }
}

pub(super) fn request_failure(error: &CompletionRequestError) -> CompletionFailure {
    CompletionFailure::new(
        400,
        Some(String::from("invalid_request_error")),
        error.to_string(),
    )
}

// The unwrapped SDK exception fails the way the route's own except does, with no type of its own
pub(super) fn transport_failure(
    failure: TextCompletionFailure,
    redactor: &Redactor,
) -> CompletionFailure {
    match failure {
        TextCompletionFailure::Wrapped(error) => wrapped_failure(&error, redactor),
        TextCompletionFailure::Raw(error) => CompletionFailure::new(500, None, error.to_string()),
    }
}

// getattr(e, "type", "None") in proxy_server.py's completion except: a null field stays null, a missing one is the literal "None"
fn exception_text(field: WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text),
        WireField::Null => None,
        WireField::Missing(_) => Some(String::from("None")),
    }
}

// The route reads type and param off the litellm exception itself, so the class metadata decides both
pub(super) fn wrapped_failure(
    error: &TextCompletionError,
    redactor: &Redactor,
) -> CompletionFailure {
    let class = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: String::new(),
            custom_llm_provider: String::from("openai"),
            error_str: error.message.clone(),
            status_code: match error.status {
                ErrorStatus::Missing | ErrorStatus::Null => None,
                ErrorStatus::Code(code) => Some(code),
            },
            status_code_is_synthesized: false,
            message: None,
            body: None,
            exception_type: String::new(),
        })),
        redactor,
    );
    let metadata = class.metadata();
    CompletionFailure {
        status: metadata.status_code,
        error_type: exception_text(metadata.error_type),
        param: exception_text(metadata.param),
        message: error.message.clone(),
    }
}
