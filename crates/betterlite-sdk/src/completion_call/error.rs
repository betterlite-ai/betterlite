use crate::call_wrapper::error::CallError;
use crate::completion_request::error::CompletionRequestError;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_core::api::chat_agentic_loop::error::AgenticLoopError;
use betterlite_core::dispatch::error::DispatchError;
use betterlite_core::operation::chat::ChatFailure;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_http::error::HttpError;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::chat::{RequestError, execution::ChatError};
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
    let (status, message) = match error {
        CompletionCallError::Dispatch(source) => dispatch_parts(source),
        other => (status_of(other), other.to_string()),
    };
    classify(model, provider, message, status, redactor)
}
fn dispatch_parts(error: &DispatchError) -> (Option<u16>, String) {
    let status = match error {
        DispatchError::Chat { source } => return chat_parts(source),
        DispatchError::DeadlineExpired { .. } => 408,
        DispatchError::Rejected { status, .. } => *status,
        DispatchError::Embedding { .. } | DispatchError::Rules { .. } => 500,
        DispatchError::MissingDeployment { .. }
        | DispatchError::NotBuilt { .. }
        | DispatchError::UnknownOperation { .. }
        | DispatchError::MismatchedOperation { .. } => 400,
    };
    (Some(status), error.to_string())
}
fn chat_parts(failure: &ChatFailure) -> (Option<u16>, String) {
    match failure {
        ChatFailure::Route(error) => {
            let status = match error {
                ChatError::Provider { response } => Some(response.status),
                ChatError::Timeout => Some(408),
                ChatError::MissingApiKey => Some(401),
                ChatError::UnknownModel { .. }
                | ChatError::Request { .. }
                | ChatError::Streaming
                | ChatError::UnsupportedProvider { .. } => Some(400),
                ChatError::Encode { .. } | ChatError::Decode { .. } | ChatError::TooLarge => {
                    Some(500)
                }
                ChatError::Http { .. } => None,
            };
            (status, error.to_string())
        }
        ChatFailure::Internal { message } => (Some(500), message.clone()),
        ChatFailure::InvalidControl { source } => (Some(400), source.to_string()),
        ChatFailure::Mismatched => (Some(500), failure.to_string()),
    }
}
fn status_of(error: &CompletionCallError) -> Option<u16> {
    match error {
        CompletionCallError::Request(CompletionRequestError::MockTimeout) => Some(408),
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
