use std::time::Instant;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use rand::Rng;
use tokio_util::sync::CancellationToken;

use crate::cooldown_decisions::failure::FailureClass;
use crate::retry_policy::eligibility::Availability;
use crate::router_core::error::RoutingError;
use crate::routing_retries::trail::RetryTrail;

// What one attempt is handed: the deployments it must not pick again and the bounds its transport call obeys.
pub struct AttemptRequest<'attempt> {
    pub skipped_deployment_ids: &'attempt [String],
    pub trail: &'attempt RetryTrail,
    // the attempt timeout, cut short by what is left of the routing deadline
    pub deadline: Instant,
    // a child of the routing token, cancelled when this attempt is abandoned
    pub cancellation: CancellationToken,
}

// One provider answer the attempt mapped to a B011 error, stamped with the deployment that gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFailure {
    pub error: ErrorClass,
    // the effective model_info id: a client-side-credential call's own id, else the deployment's
    pub deployment_id: Option<String>,
    // litellm_params.num_retries of the deployment that failed, replaced by the error's own count where the error keeps one
    pub deployment_num_retries: Option<u32>,
    // the provider's Retry-After, already parsed to whole seconds
    pub retry_after: Option<i64>,
}

#[derive(Debug)]
pub enum AttemptFailure {
    Provider(Box<ProviderFailure>),
    // no deployment could be routed, which LiteLLM never retries without a healthy deployment
    Routing(Box<RoutingError>),
}

// The class and status LiteLLM's retry checks read off an exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    pub class: FailureClass,
    // a plain exception has no status_code, while every mapped family carries one
    pub status: Option<u16>,
}

impl From<&ErrorClass> for Observed {
    fn from(error: &ErrorClass) -> Self {
        let status = match error {
            ErrorClass::Unclassified(_) => None,
            ErrorClass::Authentication(_)
            | ErrorClass::NotFound(_)
            | ErrorClass::PermissionDenied(_)
            | ErrorClass::BadRequest(_)
            | ErrorClass::UnprocessableEntity(_)
            | ErrorClass::Timeout(_)
            | ErrorClass::RateLimit(_)
            | ErrorClass::BudgetExceeded(_)
            | ErrorClass::ContextWindowExceeded(_)
            | ErrorClass::ContentPolicyViolation(_)
            | ErrorClass::ServiceUnavailable(_)
            | ErrorClass::BadGateway(_)
            | ErrorClass::InternalServer(_)
            | ErrorClass::Api(_)
            | ErrorClass::ApiConnection(_)
            | ErrorClass::ApiResponseValidation(_)
            | ErrorClass::JsonSchemaValidation(_)
            | ErrorClass::UnsupportedParams(_)
            | ErrorClass::GuardrailRaised(_)
            | ErrorClass::MidStreamFallback(_) => Some(error.metadata().status_code),
        };
        Self {
            class: FailureClass::from(error),
            status,
        }
    }
}

// The caller's half of the loop: route a fresh deployment of the logical group and dispatch it until the response is committed.
#[async_trait]
pub trait AttemptDispatch: Send {
    type Response: Send;

    // the selection draws from the same source as the backoff jitter, as LiteLLM's one random module does
    async fn attempt<R>(
        &mut self,
        request: AttemptRequest<'_>,
        random: &mut R,
    ) -> Result<Self::Response, AttemptFailure>
    where
        R: Rng + Send;

    // _async_get_healthy_deployments for the logical model group, with the fallback lists the retry check reads
    async fn availability(&mut self) -> Result<Availability, RoutingError>;

    // _handle_mock_testing_rate_limit_error: pops the request's switch and raises its rate limit before the first attempt
    fn mock_rate_limit(&mut self, model_group: &str) -> Result<(), ErrorClass>;
}
