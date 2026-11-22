#[derive(Debug, thiserror::Error)]
pub enum SummaryLimitsError<HookError> {
    #[error("the principal's budget and rate-limit fields cannot be read")]
    Principal(#[source] serde_json::Error),
    // get_proxy_hook runs before compact_20260112's protected block, so its failure reaches the caller
    #[error("the parallel request limiter hook lookup failed")]
    Hook(#[source] HookError),
}

#[derive(Debug, thiserror::Error)]
pub enum LimiterFailure<Failure> {
    // RateLimitUnverifiableError: fail_closed_rate_limit_enforcement could not verify the counters
    #[error("rate-limit counters could not be verified: {detail}")]
    Unverifiable { detail: String },
    #[error(transparent)]
    Other(Failure),
}
