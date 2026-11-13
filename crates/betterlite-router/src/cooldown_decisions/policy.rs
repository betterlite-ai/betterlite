use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cooldown_decisions::failure::{FailureClass, Seconds};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedFailsPolicy {
    #[serde(rename = "ContentPolicyViolationErrorAllowedFails")]
    pub content_policy_violation_error: Option<u32>,
    #[serde(rename = "BadRequestErrorAllowedFails")]
    pub bad_request_error: Option<u32>,
    #[serde(rename = "AuthenticationErrorAllowedFails")]
    pub authentication_error: Option<u32>,
    #[serde(rename = "TimeoutErrorAllowedFails")]
    pub timeout_error: Option<u32>,
    #[serde(rename = "RateLimitErrorAllowedFails")]
    pub rate_limit_error: Option<u32>,
    #[serde(rename = "InternalServerErrorAllowedFails")]
    pub internal_server_error: Option<u32>,
    #[serde(rename = "ServiceUnavailableErrorAllowedFails")]
    pub service_unavailable_error: Option<u32>,
    #[serde(rename = "BadGatewayErrorAllowedFails")]
    pub bad_gateway_error: Option<u32>,
    #[serde(rename = "NotFoundErrorAllowedFails")]
    pub not_found_error: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyField {
    ContentPolicyViolation,
    BadRequest,
    Authentication,
    Timeout,
    RateLimit,
    InternalServer,
    ServiceUnavailable,
    BadGateway,
    NotFound,
}

// LiteLLM's deployment-level order checks the ContentPolicyViolation subclass before its BadRequest parent
const DEPLOYMENT_ORDER: [PolicyField; 9] = [
    PolicyField::ContentPolicyViolation,
    PolicyField::BadRequest,
    PolicyField::Authentication,
    PolicyField::Timeout,
    PolicyField::RateLimit,
    PolicyField::InternalServer,
    PolicyField::ServiceUnavailable,
    PolicyField::BadGateway,
    PolicyField::NotFound,
];

// Router.get_allowed_fails_from_policy visits the same fields in a different order
const ROUTER_ORDER: [PolicyField; 9] = [
    PolicyField::Authentication,
    PolicyField::Timeout,
    PolicyField::RateLimit,
    PolicyField::ContentPolicyViolation,
    PolicyField::BadRequest,
    PolicyField::InternalServer,
    PolicyField::ServiceUnavailable,
    PolicyField::BadGateway,
    PolicyField::NotFound,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOrder {
    Deployment,
    Router,
}

impl AllowedFailsPolicy {
    #[must_use]
    pub fn resolve(&self, order: PolicyOrder, class: FailureClass) -> Option<u32> {
        let fields = match order {
            PolicyOrder::Deployment => DEPLOYMENT_ORDER,
            PolicyOrder::Router => ROUTER_ORDER,
        };
        for field in fields {
            if matches_field(class, field)
                && let Some(value) = self.value(field)
            {
                return Some(value);
            }
        }
        None
    }

    const fn value(&self, field: PolicyField) -> Option<u32> {
        match field {
            PolicyField::ContentPolicyViolation => self.content_policy_violation_error,
            PolicyField::BadRequest => self.bad_request_error,
            PolicyField::Authentication => self.authentication_error,
            PolicyField::Timeout => self.timeout_error,
            PolicyField::RateLimit => self.rate_limit_error,
            PolicyField::InternalServer => self.internal_server_error,
            PolicyField::ServiceUnavailable => self.service_unavailable_error,
            PolicyField::BadGateway => self.bad_gateway_error,
            PolicyField::NotFound => self.not_found_error,
        }
    }
}

fn matches_field(class: FailureClass, field: PolicyField) -> bool {
    match field {
        PolicyField::ContentPolicyViolation => class == FailureClass::ContentPolicyViolation,
        PolicyField::BadRequest => matches!(
            class,
            FailureClass::BadRequest
                | FailureClass::ContentPolicyViolation
                | FailureClass::ContextWindowExceeded
                | FailureClass::UnsupportedParams
        ),
        PolicyField::Authentication => class == FailureClass::Authentication,
        PolicyField::Timeout => class == FailureClass::Timeout,
        PolicyField::RateLimit => class == FailureClass::RateLimit,
        PolicyField::InternalServer => class == FailureClass::InternalServer,
        PolicyField::ServiceUnavailable => matches!(
            class,
            FailureClass::ServiceUnavailable | FailureClass::MidStreamFallback
        ),
        PolicyField::BadGateway => class == FailureClass::BadGateway,
        PolicyField::NotFound => class == FailureClass::NotFound,
    }
}

// env defaults from litellm.constants; the config crate resolves the environment once and passes the values in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureThresholds {
    pub ratio: Decimal,
    pub minimum_requests: u64,
    pub single_deployment_traffic: u64,
}

impl Default for FailureThresholds {
    fn default() -> Self {
        Self {
            ratio: Decimal::new(5, 1),
            minimum_requests: 5,
            single_deployment_traffic: 1000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooldownPolicy {
    pub allowed_fails: u32,
    pub default_allowed_fails: u32,
    pub cooldown_time: Seconds,
    pub disable_cooldowns: bool,
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
    pub thresholds: FailureThresholds,
}
