use std::sync::Arc;

use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::proxy_rate_limit_error::ProxyRateLimitProjectionError;
use betterlite_error::rate_limit::ProxyRateLimitError;
use tokio::sync::oneshot;
use tokio_util::task::TaskTracker;

use crate::concurrency_gauges::GaugeError;
use crate::fail_closed::RateLimitUnverifiableError;
use crate::identity_descriptors::agent_descriptor::AgentDescriptorBuilder;
use crate::identity_descriptors::agent_registry::AgentPolicyRegistry;
use crate::limiter_assembly::{
    CallTypeRateLimitError, CallTypeRateLimiter, RateLimitCallData, RateLimiterAssembly,
};
use crate::limiter_records::{ModelGroupResolver, RateLimitDescriptor, TagRateLimitResolver};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::project_admission::ProjectAdmissionError;
use crate::request_scopes::RequestScopeBuilder;
use crate::request_scopes::deployment_health::{DeploymentHealthPolicy, DeploymentHealthSource};
use crate::request_scopes::request_assembly::RequestAssemblyError;
// ast-grep-ignore: no-long-names - the ported LiteLLM name
use crate::request_scopes::request_assembly::build_request_rate_limit_descriptors;
use crate::request_stash::RequestStashSlot;
use crate::settlement_and_rollback::SettlementError;
use crate::token_estimation::output_cap::CapEndpoint;
use crate::token_estimation::services::{GenAiTranslation, InputTokenCounter, ResponsesInput};
use crate::token_reservations::TokenReservationError;
use crate::windowed_admission::WindowedAdmissionError;

mod capacity;
mod common;
mod flow;
mod reservation;

mod tests;

pub struct RateLimitPreCallHook {
    pub assembly: Arc<RateLimiterAssembly>,
    pub model_groups: Arc<dyn ModelGroupResolver>,
    pub agents: Arc<dyn AgentPolicyRegistry>,
    pub health: Arc<dyn DeploymentHealthSource>,
    pub error_threshold_per_minute: i64,
    pub tags: Arc<dyn TagRateLimitResolver>,
    pub translation: Arc<dyn GenAiTranslation>,
    pub responses: Arc<dyn ResponsesInput>,
    pub input_tokens: Arc<dyn InputTokenCounter>,
    pub native: Arc<dyn ProviderResolution>,
    pub router: Option<Arc<dyn DeploymentSnapshot>>,
    pub tasks: Arc<TaskTracker>,
}

// the five arguments LiteLLM's async_pre_call_hook takes, kept together so the inherent entry stays one argument
pub struct PreCallHookRequest<'request> {
    pub auth: &'request UserAPIKeyAuth,
    pub data: &'request mut RequestData,
    pub call_type: CallType,
    pub endpoint: CapEndpoint,
    pub stash: &'request RequestStashSlot,
}

// the caller keeps the guard alive for the provider request; dropping it releases the slots once the admission ends
#[must_use = "dropping the guard releases the request's capacity slot"]
pub struct RequestCapacityGuard {
    finish: Option<oneshot::Sender<()>>,
}

#[derive(Debug, thiserror::Error)]
pub enum PreCallHookError {
    #[error(transparent)]
    Descriptors(#[from] RequestAssemblyError),
    #[error(transparent)]
    Admission(#[from] WindowedAdmissionError),
    #[error(transparent)]
    Reservation(#[from] TokenReservationError),
    #[error(transparent)]
    Project(#[from] ProjectAdmissionError),
    #[error(transparent)]
    Projection(#[from] ProxyRateLimitProjectionError),
    #[error("the batch rate limiter is not registered for {call_type:?}")]
    MissingBatchLimiter { call_type: CallType },
    #[error("the request capacity admission task ended without a result")]
    AdmissionTaskLost,
    #[error("{0}")]
    RateLimited(Box<ProxyRateLimitError>),
}

impl RateLimitPreCallHook {
    // ast-grep-ignore: no-long-names - the ported LiteLLM name
    pub async fn build_request_rate_limit_descriptors(
        &self,
        user_api_key_dict: &UserAPIKeyAuth,
        data: &RequestData,
        call_type: Option<&str>,
    ) -> Result<Vec<RateLimitDescriptor>, RequestAssemblyError> {
        // ast-grep-ignore: no-long-names - the ported LiteLLM name
        build_request_rate_limit_descriptors(
            &self.request_scope_builder(),
            user_api_key_dict,
            data,
            call_type,
        )
        .await
    }

    // clippy's same_name_method refuses async_pre_call_hook while the type implements CallTypeRateLimiter
    pub async fn pre_call_hook(
        &self,
        request: PreCallHookRequest<'_>,
    ) -> Result<(), PreCallHookError> {
        tracing::debug!("Inside Rate Limit Pre-Call Hook");
        flow::run(self, request).await
    }

    pub async fn request_capacity(
        &self,
        user_api_key_dict: &UserAPIKeyAuth,
        model: &str,
        request_data: Option<&RequestData>,
    ) -> Result<RequestCapacityGuard, PreCallHookError> {
        let data = capacity::request_data_with_model(request_data, model);
        let descriptors = self
            .build_request_rate_limit_descriptors(user_api_key_dict, &data, None)
            .await?;
        let (guard, response) = capacity::start_admission(self, &descriptors).await?;
        capacity::admit_outcome(self, &descriptors, model, &response, guard)
    }

    fn request_scope_builder(&self) -> RequestScopeBuilder<'_> {
        let window_size = self.assembly.settings.window_size;
        RequestScopeBuilder {
            window_size,
            model: ModelDescriptorBuilder {
                window_size,
                resolver: Box::new(SharedModelGroups(Arc::clone(&self.model_groups))),
            },
            agents: AgentDescriptorBuilder {
                window_size,
                registry: self.agents.as_ref(),
            },
            health: DeploymentHealthPolicy {
                source: self.health.as_ref(),
                error_threshold_per_minute: self.error_threshold_per_minute,
            },
            tag_resolver: self.tags.as_ref(),
        }
    }
}

#[async_trait::async_trait]
impl CallTypeRateLimiter for RateLimitPreCallHook {
    async fn async_pre_call_hook(
        &self,
        user_api_key_dict: &UserAPIKeyAuth,
        data: &RateLimitCallData,
        call_type: CallType,
        stash: &RequestStashSlot,
    ) -> Result<(), CallTypeRateLimitError> {
        let mut owned = data.clone();
        let request = PreCallHookRequest {
            auth: user_api_key_dict,
            data: &mut owned,
            call_type,
            endpoint: CapEndpoint::Generic,
            stash,
        };
        self.pre_call_hook(request)
            .await
            .map_err(PreCallHookError::into_limiter_error)
    }
}

// the two rejection kinds a proxy turns into an HTTP status: the 429 and the fail-closed 503
pub enum PreCallHookRejection<'error> {
    RateLimited(&'error ProxyRateLimitError),
    Unverifiable(&'error RateLimitUnverifiableError),
}

impl PreCallHookError {
    // the 429 and the fail-closed 503 keep their own rejection; every other failure stays this error for the limiter to carry
    fn into_limiter_error(self) -> CallTypeRateLimitError {
        if let Some(rejection) = self.rejection() {
            return match rejection {
                PreCallHookRejection::RateLimited(rate_limited) => {
                    CallTypeRateLimitError::RateLimited(rate_limited.clone())
                }
                PreCallHookRejection::Unverifiable(unverifiable) => {
                    CallTypeRateLimitError::Unverifiable(unverifiable.clone())
                }
            };
        }
        CallTypeRateLimitError::Hook(self)
    }

    // a batch limiter reports through the same rejection types this hook raises
    fn from_limiter_error(error: CallTypeRateLimitError) -> Self {
        match error {
            CallTypeRateLimitError::RateLimited(error) => Self::RateLimited(Box::new(error)),
            CallTypeRateLimitError::Unverifiable(error) => {
                Self::Admission(WindowedAdmissionError::Unverifiable(error))
            }
            CallTypeRateLimitError::Hook(error) => error,
        }
    }

    #[must_use]
    pub fn rejection(&self) -> Option<PreCallHookRejection<'_>> {
        match self {
            Self::RateLimited(error)
            | Self::Reservation(TokenReservationError::RateLimited(error))
            | Self::Project(ProjectAdmissionError::RateLimited(error)) => {
                Some(PreCallHookRejection::RateLimited(error))
            }
            Self::Admission(WindowedAdmissionError::Unverifiable(error))
            | Self::Admission(WindowedAdmissionError::Gauges(GaugeError::Unverifiable(error)))
            | Self::Admission(WindowedAdmissionError::Settlement(SettlementError::Unverifiable(
                error,
            )))
            | Self::Reservation(TokenReservationError::Settlement(
                SettlementError::Unverifiable(error),
            ))
            | Self::Project(ProjectAdmissionError::Reservation(
                TokenReservationError::Settlement(SettlementError::Unverifiable(error)),
            )) => Some(PreCallHookRejection::Unverifiable(error)),
            Self::Descriptors(_)
            | Self::Admission(_)
            | Self::Reservation(_)
            | Self::Project(_)
            | Self::Projection(_)
            | Self::MissingBatchLimiter { .. }
            | Self::AdmissionTaskLost => None,
        }
    }
}

// ModelDescriptorBuilder owns its resolver, so a cloned Arc stands in for the shared one on every call
struct SharedModelGroups(Arc<dyn ModelGroupResolver>);

impl ModelGroupResolver for SharedModelGroups {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.resolve(model)
    }
}
