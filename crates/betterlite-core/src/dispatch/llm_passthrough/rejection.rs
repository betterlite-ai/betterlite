use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;

use super::error::{LlmPassthroughError, ModelBlockedError};
use crate::api::passthrough_call::PassthroughCallError;
use crate::dispatch::error::DispatchError;

impl LlmPassthroughError {
    // The Dispatch path's route-shaped answer; the pipeline path maps this error itself
    #[must_use]
    pub fn rejected(&self) -> DispatchError {
        let (status, message) = match self {
            Self::Call(error) => (call_status(error), error.to_string()),
            Self::Blocked(blocked) => (ModelBlockedError::STATUS.as_u16(), blocked.to_string()),
            Self::NotFound(error) => (
                ProxyModelNotFoundError::STATUS.as_u16(),
                error.detail.error.clone(),
            ),
            Self::UserConfig => (400_u16, self.to_string()),
            Self::RouterMissing => (500_u16, self.to_string()),
        };
        DispatchError::Rejected {
            status,
            error_type: String::from(error_type(status)),
            param: None,
            code: status.to_string(),
            message,
        }
    }
}

// _handle_llm_api_exception keeps the exception's status_code in 400..=599 and answers 500 otherwise
fn call_status(error: &PassthroughCallError) -> u16 {
    match error {
        PassthroughCallError::UpstreamStatus { status, .. }
        | PassthroughCallError::Routing { status, .. } => *status,
        PassthroughCallError::Resolution(_) | PassthroughCallError::MantleEndpoint(_) => 400_u16,
        PassthroughCallError::Mapped(provider) if (400..=599).contains(&provider.status_code) => {
            provider.status_code
        }
        PassthroughCallError::Mapped(_)
        | PassthroughCallError::ProviderNotFound(_)
        | PassthroughCallError::Transport(_) => 500_u16,
    }
}

const fn error_type(status: u16) -> &'static str {
    match status {
        401 => "authentication_error",
        403 => "permission_error",
        429 => "rate_limit_error",
        0..500 => "invalid_request_error",
        _ => "internal_server_error",
    }
}
