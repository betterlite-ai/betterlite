use betterlite_core::cost::usage::provider_cost::{
    hidden_response_cost, without_provider_stated_cost,
};
use betterlite_core::cost::usage::tiers::{resolve_billable_service_tier, served_service_tier};
use betterlite_core::cost::usage::{
    BillingResponse, get_usage_object, infer_call_type, is_known_usage_object,
    transcription_has_tokens,
};
use serde_json::Value;

use super::parse::{optional_mapping, optional_usage, parse_usage};
use super::projection::{decimal_value, optional_text, usage_projection};
use super::{Call, ResponseInput, failure};

pub(super) fn get_usage(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let spec = required_response(call)?;
    let owned = parse_usage(spec.usage.as_ref())?;
    let hidden = optional_mapping(spec.hidden_params.as_ref())?;
    let response = BillingResponse {
        kind: spec.kind.response_kind(),
        usage: owned.borrowed(),
        service_tier: spec.service_tier.as_deref(),
        hidden_params: hidden.as_ref(),
    };
    let usage = get_usage_object(&response).map_err(failure)?;

    Ok(usage_projection(usage.as_ref()))
}

pub(super) fn known_usage(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let spec = call
        .variant
        .as_ref()
        .ok_or_else(|| failure("variant is required"))?;
    let owned = parse_usage(Some(spec))?;

    Ok(Value::Bool(is_known_usage_object(&owned.borrowed())))
}

pub(super) fn infer(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let kind = call.kind.ok_or_else(|| failure("kind is required"))?;

    Ok(optional_text(infer_call_type(
        call.explicit_type.as_deref(),
        kind.response_kind(),
    )))
}

pub(super) fn transcription_details(
    call: &Call,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    Ok(Value::Bool(transcription_has_tokens(
        optional_usage(call)?.as_ref(),
    )))
}

pub(super) fn cleared_usage(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let Some(usage) = optional_usage(call)? else {
        return Ok(Value::Null);
    };
    let cleared = without_provider_stated_cost(&usage);

    Ok(usage_projection(Some(&cleared)))
}

pub(super) fn hidden_cost(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let hidden = optional_mapping(call.hidden_params.as_ref())?;
    let cost = hidden_response_cost(hidden.as_ref()).map_err(failure)?;

    Ok(cost.map_or(Value::Null, |amount| decimal_value(&amount)))
}

pub(super) fn served_tier(call: &Call) -> Result<Value, Box<crate::helpers::HelperError>> {
    let spec = required_response(call)?;
    let owned = parse_usage(spec.usage.as_ref())?;
    let hidden = optional_mapping(spec.hidden_params.as_ref())?;
    let response = BillingResponse {
        kind: spec.kind.response_kind(),
        usage: owned.borrowed(),
        service_tier: spec.service_tier.as_deref(),
        hidden_params: hidden.as_ref(),
    };
    let usage = optional_usage(call)?;
    let tier = served_service_tier(&response, usage.as_ref());

    Ok(optional_text(tier.as_deref()))
}

pub(super) fn resolve_tier(call: &Call) -> Value {
    let resolved =
        resolve_billable_service_tier(call.requested_tier.as_deref(), call.served_tier.as_deref());

    optional_text(resolved.as_deref())
}

fn required_response(call: &Call) -> Result<&ResponseInput, Box<crate::helpers::HelperError>> {
    call.response
        .as_ref()
        .ok_or_else(|| failure("response is required"))
}
