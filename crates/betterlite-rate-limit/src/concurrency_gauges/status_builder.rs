use crate::limiter_records::{
    ParallelRequestGauge, RateLimitResponse, RateLimitStatus, RateLimitType,
};

mod tests;

pub(super) fn gauge_status(
    gauge: &ParallelRequestGauge,
    in_flight: i64,
    code: &str,
) -> RateLimitStatus {
    RateLimitStatus {
        code: code.to_owned(),
        current_limit: gauge.limit,
        limit_remaining: gauge.limit.saturating_sub(in_flight).max(0),
        rate_limit_type: RateLimitType::MaxParallelRequests,
        descriptor_key: gauge.descriptor_key.clone(),
        descriptor_value: None,
    }
}

pub(super) fn over_limit_response(
    gauge: &ParallelRequestGauge,
    in_flight: i64,
) -> RateLimitResponse {
    RateLimitResponse {
        overall_code: "OVER_LIMIT".to_owned(),
        statuses: vec![gauge_status(gauge, in_flight, "OVER_LIMIT")],
        reservation_windows: None,
    }
}
