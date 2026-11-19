use betterlite_rate_limit::concurrency_gauges::GaugeError;
use betterlite_rate_limit::limiter_records::RateLimitResponse;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn gauge_error(error: &GaugeError) -> Box<HelperError> {
    let status = match error {
        GaugeError::Unverifiable(unverifiable) => Some(unverifiable.status_code),
        GaugeError::Reply(_)
        | GaugeError::NotWholeNumber { .. }
        | GaugeError::MissingAcquireReply
        | GaugeError::ShortAcquireReply { .. }
        | GaugeError::GaugeIndex { .. }
        | GaugeError::ClockBeforeEpoch(_)
        | GaugeError::Redis(_) => None,
    };
    let class = if status.is_some() {
        "RateLimitUnverifiableError"
    } else {
        "ValueError"
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(super) fn response_json(response: &RateLimitResponse) -> Value {
    let statuses: Vec<Value> = response
        .statuses
        .iter()
        .map(|status| {
            json!({
                "code": status.code,
                "current_limit": status.current_limit,
                "limit_remaining": status.limit_remaining,
                "rate_limit_type": status.rate_limit_type.as_str(),
                "descriptor_key": status.descriptor_key,
            })
        })
        .collect();
    json!({
        "overall_code": response.overall_code,
        "statuses": statuses,
        "reservation_windows": [],
    })
}
