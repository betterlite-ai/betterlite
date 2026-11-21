#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitType {
    Requests,
    Tokens,
}

impl RateLimitType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::Tokens => "tokens",
        }
    }
}

// the two entities the v3 limiter tracks a limit for
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitDescriptor {
    ApiKey,
    Team,
}

impl RateLimitDescriptor {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Team => "team",
        }
    }
}

// one gauge a rate-limit emitter writes, named as the PrometheusLogger attribute it holds
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitMetric {
    ApiKeyRateLimitAllowed,
    ApiKeyRateLimitUsed,
    TeamRateLimitAllowed,
    TeamRateLimitUsed,
    RemainingApiKeyRequests,
    RemainingApiKeyTokens,
    DeploymentTpmLimit,
    DeploymentRpmLimit,
    RemainingRequests,
    RemainingTokens,
}

impl RateLimitMetric {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ApiKeyRateLimitAllowed => "litellm_api_key_rate_limit_allowed_metric",
            Self::ApiKeyRateLimitUsed => "litellm_api_key_rate_limit_used_metric",
            Self::TeamRateLimitAllowed => "litellm_team_rate_limit_allowed_metric",
            Self::TeamRateLimitUsed => "litellm_team_rate_limit_used_metric",
            Self::RemainingApiKeyRequests => "litellm_remaining_api_key_requests_for_model",
            Self::RemainingApiKeyTokens => "litellm_remaining_api_key_tokens_for_model",
            Self::DeploymentTpmLimit => "litellm_deployment_tpm_limit",
            Self::DeploymentRpmLimit => "litellm_deployment_rpm_limit",
            Self::RemainingRequests => "litellm_remaining_requests_metric",
            Self::RemainingTokens => "litellm_remaining_tokens_metric",
        }
    }
}
