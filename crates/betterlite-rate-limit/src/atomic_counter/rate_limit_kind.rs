use crate::limiter_records::{RateLimitType, WindowRateLimitType};

#[must_use]
pub const fn status_rate_limit_type(kind: WindowRateLimitType) -> RateLimitType {
    match kind {
        WindowRateLimitType::Requests => RateLimitType::Requests,
        WindowRateLimitType::Tokens => RateLimitType::Tokens,
    }
}
