pub mod entry;
pub mod route;

use async_trait::async_trait;
use betterlite_context::call_context::CallContext;
use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use http::StatusCode;

use super::error::DispatchError;
use crate::api::passthrough_call::{
    PassthroughCallArgs, PassthroughCallError, PassthroughReply, PassthroughRouterCalls,
};
use crate::operation::llm_passthrough::{LlmPassthroughRequest, LlmPassthroughResult};

mod tests;

// The router's blocked check: get_model_list for the model and team, then _are_all_deployments_blocked
pub trait BlockedModels: Send + Sync {
    fn all_blocked(&self, model: &str, team_id: Option<&str>) -> bool;
}

// The call port the plan hands the args to; litellm.allm_passthrough_route needs the logging state and HTTP pool this crate does not hold
#[async_trait]
pub trait PassthroughCallPort: Send + Sync {
    async fn call(
        &self,
        args: PassthroughCallArgs,
    ) -> Result<PassthroughReply, PassthroughCallError>;
}

// B8690's PassthroughRouterCalls has an impl Future return no trait object can hold, so its ports get the boxed form
#[async_trait]
impl<Calls> PassthroughCallPort for Calls
where
    Calls: PassthroughRouterCalls,
{
    async fn call(
        &self,
        args: PassthroughCallArgs,
    ) -> Result<PassthroughReply, PassthroughCallError> {
        self.allm_passthrough_route(args).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Model is blocked")]
pub struct BlockedModel {
    pub model: String,
}

impl BlockedModel {
    pub const STATUS: StatusCode = StatusCode::FORBIDDEN;
}

#[derive(Debug, thiserror::Error)]
pub enum LlmPassthroughError {
    #[error(transparent)]
    Call(#[from] PassthroughCallError),
    #[error(transparent)]
    Blocked(#[from] BlockedModel),
    #[error(transparent)]
    NotFound(#[from] ProxyModelNotFoundError),
    #[error("user_config is not supported on pass-through routes")]
    UserConfig,
    #[error("the route chose the router, but the request carries no router port")]
    RouterMissing,
}

impl LlmPassthroughError {
    // The Dispatch path's route-shaped answer; the pipeline path maps this error itself
    #[must_use]
    pub fn rejected(&self) -> DispatchError {
        match self {
            Self::Call(error) => call_rejected(error),
            Self::Blocked(blocked) => {
                let status = BlockedModel::STATUS.as_u16();
                route_error(status, error_type(status), blocked.to_string())
            }
            Self::NotFound(error) => {
                let status = ProxyModelNotFoundError::STATUS.as_u16();
                route_error(status, error_type(status), error.detail.error.clone())
            }
            Self::UserConfig => route_error(400_u16, error_type(400_u16), self.to_string()),
            Self::RouterMissing => route_error(500_u16, error_type(500_u16), self.to_string()),
        }
    }
}

fn route_error(status: u16, error_type: &str, message: String) -> DispatchError {
    DispatchError::Rejected {
        status,
        error_type: String::from(error_type),
        param: None,
        code: status.to_string(),
        message,
    }
}

fn call_rejected(error: &PassthroughCallError) -> DispatchError {
    let status = match error {
        PassthroughCallError::UpstreamStatus { status, .. }
        | PassthroughCallError::Routing { status, .. } => *status,
        PassthroughCallError::ProviderNotFound(_)
        | PassthroughCallError::Resolution(_)
        | PassthroughCallError::Mapped(_)
        | PassthroughCallError::Transport(_) => 500_u16,
    };
    route_error(status, error_type(status), error.to_string())
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

pub async fn execute<Metadata, Client>(
    request: LlmPassthroughRequest,
    _context: &CallContext<'_, Metadata, Client>,
) -> Result<LlmPassthroughResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    match entry::execute(request).await {
        Ok(reply) => Ok(reply),
        Err(error) => Err(error.rejected()),
    }
}
