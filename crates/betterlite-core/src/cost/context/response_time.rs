use std::str::FromStr as _;

use betterlite_logging::fields::timestamps::{Moment, get_response_ms};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::CostResponse;
use super::error::BillingContextError;

pub struct ResponseTiming<'timing> {
    pub start_time: Option<&'timing Moment>,
    pub end_time: Option<&'timing Moment>,
    pub now: &'timing Moment,
}

// Ports _response_time_ms_for_cost: a stamped numeric _response_ms wins even at zero, then a nonzero total_time.
pub fn response_time_ms(
    response: Option<&CostResponse<'_>>,
    total_time: Option<Decimal>,
    timing: Option<&ResponseTiming<'_>>,
) -> Result<Decimal, BillingContextError> {
    if let Some(CostResponse::Object {
        response_ms: Some(stamped),
        ..
    }) = response
    {
        match stamped {
            Json::Bool(flag) => return Ok(Decimal::from(u8::from(*flag))),
            Json::Number(number) => {
                return Decimal::from_str(&number.to_string())
                    .map_err(|source| BillingContextError::StampedTime { source });
            }
            Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => {}
        }
    }
    if let Some(total) = total_time.filter(|value| !value.is_zero()) {
        return Ok(total);
    }
    let Some(clock) = timing else {
        return Ok(Decimal::ZERO);
    };
    get_response_ms(clock.start_time, clock.end_time, clock.now).map_err(BillingContextError::from)
}
