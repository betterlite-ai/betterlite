use std::ops::Div as _;

use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_error::proxy_rate_limit_error::{
    ProxyRateLimitProjectionError, RateLimitResetWindow, V3RateLimitExceeded, v3_rate_limit_error,
};
use betterlite_error::rate_limit::ProxyRateLimitError;
use num_traits::cast::cast;
use serde::Deserialize;
use serde_json::Number;

use crate::limiter_records::{RateLimitDescriptor, RateLimitResponse};

mod tests;

// litellm.types.utils.PriorityReservationDict's "type": an absent or null key is percent, and an unknown name takes the percent fallback too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityKind {
    Percent,
    Rpm,
    Tpm,
}

impl PriorityKind {
    fn from_wire(text: Option<&str>) -> Self {
        match text {
            Some("rpm") => Self::Rpm,
            Some("tpm") => Self::Tpm,
            Some(_) | None => Self::Percent,
        }
    }
}

// PriorityReservationDict as LiteLLM reads it: both keys are optional, and an explicit null counts as absent (docs/compatibility.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriorityReservation {
    pub kind: PriorityKind,
    pub value: f64,
}

#[derive(Deserialize)]
struct PriorityReservationRaw {
    #[serde(rename = "type")]
    kind: Option<String>,
    value: Option<Number>,
}

// Why a priority value can't be read: a JSON number no float can hold.
#[derive(Debug, thiserror::Error)]
pub enum PriorityValueError {
    #[error("the priority value is not a finite number")]
    NotANumber,
}

// The two ModelGroupInfo capacities a priority reservation can be measured against.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct ModelGroupLimits {
    pub rpm: Option<i64>,
    pub tpm: Option<i64>,
}

// convert_priority_to_percent's argument: a plain number is already a fraction of capacity, a mapping says what its value counts.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(try_from = "PriorityValueRaw")]
pub enum PriorityValue {
    Percent(f64),
    Reservation(PriorityReservation),
}

// serde_json's arbitrary_precision hands a float to serde's content buffer as {"$serde_json::private::Number": "0.9"}, and Number reads that shape back.
#[derive(Deserialize)]
#[serde(untagged)]
enum PriorityValueRaw {
    Percent(Number),
    Reservation(PriorityReservationRaw),
}

impl TryFrom<PriorityValueRaw> for PriorityValue {
    type Error = PriorityValueError;

    fn try_from(raw: PriorityValueRaw) -> Result<Self, Self::Error> {
        match raw {
            PriorityValueRaw::Percent(number) => Ok(Self::Percent(
                number.as_f64().ok_or(PriorityValueError::NotANumber)?,
            )),
            PriorityValueRaw::Reservation(reservation) => {
                let value = reservation.value.map_or(Ok(1.0), |number| {
                    number.as_f64().ok_or(PriorityValueError::NotANumber)
                })?;
                Ok(Self::Reservation(PriorityReservation {
                    kind: PriorityKind::from_wire(reservation.kind.as_deref()),
                    value,
                }))
            }
        }
    }
}

// convert_priority_to_percent: a percent passes through, and an rpm or tpm value is divided by the model group's capacity when that capacity is a positive limit.
#[must_use]
pub fn convert_priority_to_percent(
    value: PriorityValue,
    model_info: Option<&ModelGroupLimits>,
) -> f64 {
    match value {
        PriorityValue::Percent(percent) => percent,
        PriorityValue::Reservation(reservation) => reservation_percent(reservation, model_info),
    }
}

fn reservation_percent(
    reservation: PriorityReservation,
    model_info: Option<&ModelGroupLimits>,
) -> f64 {
    let capacity = match reservation.kind {
        PriorityKind::Percent => return reservation.value,
        PriorityKind::Rpm => model_info.and_then(|limits| limits.rpm),
        PriorityKind::Tpm => model_info.and_then(|limits| limits.tpm),
    };
    let positive = capacity
        .and_then(cast::<i64, f64>)
        .filter(|limit| *limit > 0.0_f64);
    positive.map_or(reservation.value, |limit| reservation.value.div(limit))
}

// What the rejected request's provider attribution needs: B2327's native resolver and the router its alias fallback reads (None before startup builds one).
pub struct RateLimitErrorAttribution<'request> {
    pub requested_model: Option<&'request str>,
    pub native: &'request dyn ProviderResolution,
    pub router: Option<&'request dyn DeploymentSnapshot>,
}

// _handle_rate_limit_error: the first OVER_LIMIT status becomes the 429 the v3 limiter raises, and a response with none raises nothing.
pub fn handle_rate_limit_error(
    response: &RateLimitResponse,
    descriptors: &[RateLimitDescriptor],
    window: RateLimitResetWindow,
    attribution: &RateLimitErrorAttribution<'_>,
) -> Result<Option<ProxyRateLimitError>, ProxyRateLimitProjectionError> {
    let Some(status) = response
        .statuses
        .iter()
        .find(|status| status.code == "OVER_LIMIT")
    else {
        return Ok(None);
    };
    let descriptor_value = descriptors
        .iter()
        .find(|descriptor| {
            descriptor.key == status.descriptor_key
                && status
                    .descriptor_value
                    .as_deref()
                    .is_none_or(|value| descriptor.value == value)
        })
        .map_or("unknown", |descriptor| descriptor.value.as_str());
    let (model, llm_provider) = resolve_rate_limit_provider(
        attribution.requested_model,
        attribution.native,
        attribution.router,
    );
    let exceeded = V3RateLimitExceeded {
        descriptor_key: status.descriptor_key.as_str(),
        descriptor_value,
        rate_limit_type: status.rate_limit_type.as_str(),
        current_limit: status.current_limit,
        limit_remaining: status.limit_remaining,
    };
    v3_rate_limit_error(&exceeded, window, model, llm_provider).map(Some)
}
