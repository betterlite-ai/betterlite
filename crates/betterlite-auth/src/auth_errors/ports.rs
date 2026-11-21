use std::time::Duration;

use async_trait::async_trait;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{ResolutionError, ResolvedModel};

use super::conversion::AuthFailure;
use crate::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use crate::resolved_key::auth::UserAPIKeyAuth;

// verbose_proxy_logger, or its stdout child that the malformed-key warnings use
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Logger {
    Proxy,
    ProxyStdout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthLog {
    pub logger: Logger,
    pub level: Level,
    pub message: String,
    pub traceback: bool,
    pub requester_ip: Option<String>,
}

pub struct ServiceFailure<'failure> {
    pub error: &'failure AuthFailure,
    pub call_type: &'static str,
    pub duration: Duration,
}

pub struct FailureHookCall<'call> {
    pub request_data: &'call JsonObject,
    pub original: &'call AuthFailure,
    pub identity: &'call UserAPIKeyAuth,
    pub error_type: ProxyErrorTypes,
    pub route: &'call str,
}

// the callback's own error stays with the callback, so nothing it carried can reach a log line or the client
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("a post-call failure callback raised")]
    Raised,
}

// a caller without the router's provider table: every budget rejection falls back to litellm_proxy
pub struct NoNativeProvider;

impl ProviderResolution for NoNativeProvider {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

#[async_trait]
pub trait AuthErrorPorts: Send + Sync {
    fn record_service_failure(&self, failure: &ServiceFailure<'_>);

    fn log(&self, entry: AuthLog);

    fn seed_identity(&self, identity: &UserAPIKeyAuth, model: Option<&Json>);

    fn native_resolution(&self) -> &dyn ProviderResolution;

    fn router(&self) -> Option<&dyn DeploymentSnapshot>;

    async fn post_call_failure_hook(
        &self,
        call: FailureHookCall<'_>,
    ) -> Result<Option<AuthFailure>, HookError>;
}
