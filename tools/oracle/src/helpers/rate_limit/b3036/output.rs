use betterlite_rate_limit::limiter_records::RateLimitResponse;
use betterlite_rate_limit::windowed_admission::WindowedAdmissionError;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn over_limit_error(error: &WindowedAdmissionError) -> Box<HelperError> {
    let status = match error {
        WindowedAdmissionError::Unverifiable(unverifiable) => Some(unverifiable.status_code),
        WindowedAdmissionError::ClockBeforeEpoch(_)
        | WindowedAdmissionError::ClockOverflow { .. }
        | WindowedAdmissionError::WindowStore(_)
        | WindowedAdmissionError::Gauges(_)
        | WindowedAdmissionError::Settlement(_)
        | WindowedAdmissionError::Reply(_)
        | WindowedAdmissionError::Counter(_)
        | WindowedAdmissionError::Batch(_)
        | WindowedAdmissionError::WindowPairCount { .. }
        | WindowedAdmissionError::MissingWindowMetadata { .. } => None,
    };
    Box::new(HelperError {
        class: "ValueError".to_owned(),
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
                "descriptor_value": status.descriptor_value,
            })
        })
        .collect();
    json!({
        "overall_code": response.overall_code,
        "statuses": statuses,
        "reservation_windows": response.reservation_windows,
    })
}
