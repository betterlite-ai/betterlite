use crate::call_wrapper::error::CallError;
use crate::completion_request::error::CompletionRequestError;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_core::api::chat_agentic_loop::error::AgenticLoopError;
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::operation::chat::ChatFailure;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_http::error::HttpError;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::chat::RequestError;
use betterlite_openai::chat::execution::ChatError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum CompletionCallError {
    #[error(transparent)]
    Request(#[from] CompletionRequestError),
    #[error(transparent)]
    ChatRequest(#[from] RequestError),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error(transparent)]
    Body(#[from] serde_json::Error),
    #[error("{message}")]
    Conversion { message: String },
    #[error(transparent)]
    Agentic(#[from] AgenticLoopError),
    #[error(transparent)]
    Mock(#[from] MockError),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Call(#[from] CallError),
    // main.py's palm branch raises a ValueError with this text
    #[error(
        "Palm was decommi\u{73}ioned on October 2024. Please use the `gemini/` route for Gemini Google AI Studio Models. Announcement: https://ai.google.dev/palm_docs/palm?hl=en"
    )]
    Palm,
    #[error("the MCP gateway loop is not available for this call")]
    NoMcpLoop,
    #[error("the call answered with a result of another kind: {entry}")]
    UnexpectedResult { entry: &'static str },
    #[error(
        "No response from fallbacks. Got none. Turn on `litellm.set_verbose=True` to see more details."
    )]
    NoFallbackResponse,
    #[error(
        "{last}. All fallback attempts failed. Enable verbose logging with `litellm.set_verbose=True` for details."
    )]
    AllFallbacksFailed { last: String },
    #[error("the mock call carries no mock response to answer with")]
    MissingMock,
    #[error("the model list answered nothing")]
    NoModelListAnswer,
    #[error("the {field} value is not valid here")]
    Field { field: &'static str },
}
#[must_use]
pub fn into_call_error(
    error: &CompletionCallError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallError {
    if let CompletionCallError::Dispatch(source) = error {
        return dispatch_error(source, model, provider, redactor);
    }
    classify(
        model,
        provider,
        error.to_string(),
        status_of(error),
        redactor,
    )
}
fn dispatch_error(
    error: &DispatchError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallError {
    if let DispatchError::Chat { source } = error {
        return chat_failure(source, model, provider, redactor);
    }
    let status = match error {
        DispatchError::DeadlineExpired { .. } => 408,
        DispatchError::Rejected { status, .. } => *status,
        DispatchError::Chat { .. }
        | DispatchError::Embedding { .. }
        | DispatchError::Rules { .. } => 500,
        DispatchError::MissingDeployment { .. }
        | DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. } => 400,
    };
    classify(model, provider, error.to_string(), Some(status), redactor)
}
fn chat_failure(
    failure: &ChatFailure,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallError {
    match failure {
        ChatFailure::Route(error) => classify(
            model,
            provider,
            error.to_string(),
            match error {
                ChatError::Timeout => Some(408),
                ChatError::Provider { response } => Some(response.status),
                ChatError::MissingApiKey => Some(401),
                ChatError::UnknownModel { .. }
                | ChatError::Request { .. }
                | ChatError::Streaming
                | ChatError::UnsupportedProvider { .. } => Some(400),
                ChatError::Encode { .. } | ChatError::Decode { .. } | ChatError::TooLarge => {
                    Some(500)
                }
                ChatError::Http { .. } => None,
            },
            redactor,
        ),
        ChatFailure::Internal { message } => {
            classify(model, provider, message.clone(), Some(500), redactor)
        }
        ChatFailure::InvalidControl { source } => {
            classify(model, provider, source.to_string(), Some(400), redactor)
        }
        ChatFailure::Mismatched => {
            classify(model, provider, failure.to_string(), Some(500), redactor)
        }
    }
}
fn status_of(error: &CompletionCallError) -> Option<u16> {
    match error {
        CompletionCallError::Request(_)
        | CompletionCallError::ChatRequest(_)
        | CompletionCallError::Field { .. }
        | CompletionCallError::MissingMock
        | CompletionCallError::NoMcpLoop => Some(400),
        CompletionCallError::NoFallbackResponse
        | CompletionCallError::AllFallbacksFailed { .. }
        | CompletionCallError::NoModelListAnswer => Some(500),
        CompletionCallError::Mock(source) => Some(source.status().unwrap_or(500)),
        CompletionCallError::Dispatch(_)
        | CompletionCallError::Palm
        | CompletionCallError::Http(_)
        | CompletionCallError::Body(_)
        | CompletionCallError::Conversion { .. }
        | CompletionCallError::Agentic(_)
        | CompletionCallError::Resolution(_)
        | CompletionCallError::Call(_)
        | CompletionCallError::UnexpectedResult { .. } => None,
    }
}
fn classify(
    model: &str,
    provider: &str,
    message: String,
    status: Option<u16>,
    redactor: &Redactor,
) -> CallError {
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code: status,
        status_code_is_synthesized: status.is_none(),
        message: Some(message),
        body: None,
        exception_type: String::new(),
    };
    CallError::from(dispatch(&DispatchInput::Raw(Box::new(raw)), redactor))
}
