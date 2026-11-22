use betterlite_models::decisions::Json;

use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_usage::usage_object::integer;

use super::error::RateLimitValueError;
use super::metric::RateLimitType;

// the v3 parallel-request limiter writes this family for this request's model group, with no model group in the name
const V3_MODEL_PER_KEY: &str = "x-ratelimit-model_per_key-remaining-";

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_remaining_from_v3_rate_limit_headers(
    payload: Option<&StandardLoggingPayload>,
    rate_limit_type: RateLimitType,
) -> Option<i64> {
    let logged_payload = payload?;
    let header_name = format!("{V3_MODEL_PER_KEY}{}", rate_limit_type.as_str());
    // ast-grep-ignore: no-long-names
    get_int_from_v3_rate_limit_headers(logged_payload, &header_name)
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_int_from_v3_rate_limit_headers(
    payload: &StandardLoggingPayload,
    header_name: &str,
) -> Option<i64> {
    let additional_headers = payload.hidden_params.additional_headers.as_ref()?;
    let value = additional_headers.get(header_name)?;
    // integer() matches only a JSON number, so a bool or a float reads as None like LiteLLM's isinstance checks
    integer(Some(value))
}

// an integer a caller declared, so a present value of another JSON type is an error that names the field instead of a silent default
pub(crate) fn declared_integer(
    value: Option<&Json>,
    field: &str,
) -> Result<Option<i64>, RateLimitValueError> {
    let amount = match value {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Number(number)) => number.as_i64(),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => None,
    };
    amount
        .map(Some)
        .ok_or_else(|| RateLimitValueError::NotAnInteger {
            field: field.to_owned(),
        })
}
