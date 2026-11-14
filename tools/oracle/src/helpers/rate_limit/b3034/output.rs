use betterlite_rate_limit::limiter_records::{RateLimitResponse, ReservationWindowIdentity};
use betterlite_rate_limit::settlement_and_rollback::SettlementError;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(super) fn settlement_error(error: SettlementError) -> Box<HelperError> {
    let status = match &error {
        SettlementError::Unverifiable(unverifiable) => Some(unverifiable.status_code),
        SettlementError::DescriptorCount { .. }
        | SettlementError::Reply(_)
        | SettlementError::Response(_)
        | SettlementError::AtomicCounter(_)
        | SettlementError::WindowCounter(_)
        | SettlementError::ClockBeforeEpoch(_)
        | SettlementError::ClockOverflow { .. }
        | SettlementError::UnpairedRefundKeys { .. }
        | SettlementError::RefundPosition { .. } => None,
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
    let windows: Vec<&ReservationWindowIdentity> =
        response.reservation_windows.iter().flatten().collect();
    json!({
        "overall_code": response.overall_code.clone(),
        "statuses": response.statuses.clone(),
        "reservation_windows": windows,
    })
}
