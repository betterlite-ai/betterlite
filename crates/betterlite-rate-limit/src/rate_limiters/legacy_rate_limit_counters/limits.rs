use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_error::rate_limit::RateLimitDimension;

use super::usage_cache::MinuteCounter;

// the three limits check_key_in_limits compares against, already defaulted and throttled by the caller
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DimensionLimits {
    pub max_parallel_requests: i64,
    pub tpm_limit: i64,
    pub rpm_limit: i64,
}

// check_key_in_limits' rate_limit_type: the literal LiteLLM writes into the detail and the log line
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitScope {
    Key,
    ModelPerKey,
    User,
    Customer,
    Team,
}

impl RateLimitScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::ModelPerKey => "model_per_key",
            Self::User => "user",
            Self::Customer => "customer",
            Self::Team => "team",
        }
    }
}

// one check's inputs, so the async call keeps to two parameters
pub struct KeyLimitCheck<'request> {
    pub limits: DimensionLimits,
    pub current: Option<MinuteCounter>,
    pub request_count_api_key: String,
    pub rate_limit_type: RateLimitScope,
    pub requested_model: Option<&'request str>,
    pub parent_otel_span: Option<&'request ParentOtelSpan>,
}

impl DimensionLimits {
    // LiteLLM's zero-limit precedence: max_parallel_requests, then tpm, then rpm
    #[must_use]
    pub const fn disabled_dimension(self) -> RateLimitDimension {
        if self.max_parallel_requests == 0 {
            RateLimitDimension::ConcurrentRequests
        } else if self.tpm_limit == 0 {
            RateLimitDimension::Tokens
        } else {
            RateLimitDimension::Requests
        }
    }

    // LiteLLM's over-limit precedence, matching the order of the pass condition
    #[must_use]
    pub const fn tripped_dimension(self, current: MinuteCounter) -> RateLimitDimension {
        if current.current_requests >= self.max_parallel_requests {
            RateLimitDimension::ConcurrentRequests
        } else if current.current_tpm >= self.tpm_limit {
            RateLimitDimension::Tokens
        } else {
            RateLimitDimension::Requests
        }
    }
}
