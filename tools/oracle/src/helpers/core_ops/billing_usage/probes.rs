use betterlite_core::cost::usage::tiers::served_service_tier;
use betterlite_core::cost::usage::{BillingResponse, get_usage_object};
use serde_json::Value;

use super::model::{
    optional_mapping, optional_text, optional_usage, parse_usage, usage_projection,
};
use super::{Call, ResponseInput, failure};

pub(super) fn get_usage(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let spec = required_response(call)?;
    let owned = parse_usage(spec.usage.as_ref())?;
    let hidden = optional_mapping(spec.hidden_params.as_ref())?;
    let usage = get_usage_object(&BillingResponse {
        kind: spec.kind.response_kind(),
        usage: owned.borrowed(),
        service_tier: spec.service_tier.as_deref(),
        hidden_params: hidden.as_ref(),
    })
    .map_err(failure)?;

    Ok(usage_projection(usage.as_ref()))
}

// The source reads the served tier from the response and the normalized usage, never from the response usage object.
pub(super) fn served_tier(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let input = call
        .response
        .as_ref()
        .ok_or_else(|| failure("response is required"))?;
    Ok(optional_text(
        served_service_tier(
            input.service_tier.as_deref(),
            optional_mapping(input.hidden_params.as_ref())?.as_ref(),
            optional_usage(call)?.as_ref(),
        )
        .as_deref(),
    ))
}

pub(super) fn required_response(
    call: &Call,
) -> Result<&ResponseInput, Box<crate::helpers::HelperError>> {
    call.response
        .as_ref()
        .ok_or_else(|| failure("response is required"))
}
