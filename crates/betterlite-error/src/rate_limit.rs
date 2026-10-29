mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitCategory {
    VendorRateLimit,
    VendorBatchRateLimit,
    LitellmRateLimit,
    LitellmBatchRateLimit,
}

impl RateLimitCategory {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::VendorRateLimit => "vendor_rate_limit",
            Self::VendorBatchRateLimit => "vendor_batch_rate_limit",
            Self::LitellmRateLimit => "litellm_rate_limit",
            Self::LitellmBatchRateLimit => "litellm_batch_rate_limit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitDimension {
    Requests,
    Tokens,
    ConcurrentRequests,
    Budget,
    MaxIterations,
}

impl RateLimitDimension {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::Tokens => "tokens",
            Self::ConcurrentRequests => "concurrent_requests",
            Self::Budget => "budget",
            Self::MaxIterations => "max_iterations",
        }
    }
}
