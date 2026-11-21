use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use http::StatusCode;

use crate::api::passthrough_call::PassthroughCallError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Model is blocked")]
pub struct ModelBlockedError {
    pub model: String,
}

impl ModelBlockedError {
    pub const STATUS: StatusCode = StatusCode::FORBIDDEN;
}

#[derive(Debug, thiserror::Error)]
pub enum LlmPassthroughError {
    #[error(transparent)]
    Call(#[from] PassthroughCallError),
    #[error(transparent)]
    Blocked(#[from] ModelBlockedError),
    #[error(transparent)]
    NotFound(#[from] ProxyModelNotFoundError),
    #[error("user_config is not supported on pass-through routes")]
    UserConfig,
    #[error("the route chose the router, but the request carries no router port")]
    RouterMissing,
}
