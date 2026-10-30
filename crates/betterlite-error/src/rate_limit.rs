mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitCategory {
    VendorRateLimit,
    VendorBatchRateLimit,
    LitellmRateLimit,
    LitellmBatchRateLimit,
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
