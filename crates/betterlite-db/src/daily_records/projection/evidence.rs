use serde_json::Number;

use betterlite_models::savings_evidence::billed_at::BilledAt;
use betterlite_models::savings_evidence::cache_extraction::{
    extract_cache_creation_tokens, extract_cache_read_tokens,
};
use betterlite_models::savings_evidence::compression::compression_saved_tokens;
use betterlite_models::savings_evidence::injection::marks_gateway_injection;
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use betterlite_models::savings_pricing::spend::{SavingsSpendInput, compute_savings_spend};
use betterlite_models::savings_pricing::usage::request_usage;
use betterlite_models::tiered_pricing::Scalar;

use super::super::DailyRecordError;
use super::super::metadata::{DailyMetadata, RequestStatus, request_status};
use super::presence;
use super::{CallTypeEndpoints, DailyEvidence, DailyPayload, DailyPricing};

// Every field of the base transaction that comes from the metadata or the payload's own call evidence.
pub(super) async fn gather<C>(
    payload: &DailyPayload,
    metadata: &DailyMetadata,
    pricing: &DailyPricing<'_, C>,
    endpoints: &dyn CallTypeEndpoints,
) -> Result<DailyEvidence, DailyRecordError>
where
    C: SavingsPricingContext,
{
    let internal = metadata.internal_call;
    let status = request_status(payload.metadata.as_deref().map(str::as_bytes));
    let cache_read = extract_cache_read_tokens(metadata.usage_object.as_ref())?;
    let cache_creation = extract_cache_creation_tokens(metadata.usage_object.as_ref())?;
    let compression_saved = compression_saved_tokens(&metadata.savings)?;
    let savings = savings_spend(payload, metadata, pricing, compression_saved).await?;
    let timed = timed_duration_ms(payload, status, internal);
    Ok(DailyEvidence {
        status,
        internal,
        endpoint: call_endpoint(payload, endpoints),
        cache_read,
        cache_creation,
        compression_saved,
        savings,
        timed,
    })
}

// compute_savings_spend: the same evidence the request's own savings record was built from, priced again for the rollup row.
async fn savings_spend<C>(
    payload: &DailyPayload,
    metadata: &DailyMetadata,
    pricing: &DailyPricing<'_, C>,
    compression_saved: i64,
) -> Result<SavingsSpend, DailyRecordError>
where
    C: SavingsPricingContext,
{
    let usage = request_usage(metadata.raw_usage.as_ref());
    let billed_at = billed_at(payload)?;
    Ok(compute_savings_spend(
        pricing.context,
        &SavingsSpendInput {
            model: presence(payload.model.as_ref()),
            custom_llm_provider: presence(payload.custom_llm_provider.as_ref()),
            compression_saved_tokens: compression_saved,
            gateway_injected_cache: marks_gateway_injection(
                Some(&metadata.savings),
                payload.model_id.as_deref(),
            ),
            routing_decision: metadata.routing_decision.as_ref(),
            usage: usage.as_ref(),
            model_id: payload.model_id.as_deref(),
            cost_breakdown: metadata.cost_breakdown.as_ref(),
            recorded_autorouter_savings: metadata.savings.autorouter_savings.as_ref(),
            recorded_estimate: metadata.savings.autorouter_savings_estimate.as_ref(),
            billed_at: &billed_at,
            fallback_moment: pricing.fallback_moment,
            database: pricing.zone,
        },
    )
    .await)
}

// payload.get("endTime") through _coerce_billed_at: only the logged text is a bill time here, and a value it cannot read fails the row the way its AttributeError does.
fn billed_at(payload: &DailyPayload) -> Result<BilledAt, DailyRecordError> {
    match payload.end_time.as_ref() {
        None | Some(Scalar::Null) => Ok(BilledAt::Missing),
        Some(Scalar::Text(text)) => Ok(BilledAt::Text(text.clone())),
        Some(Scalar::Bool(_) | Scalar::Number(_) | Scalar::Array(_) | Scalar::Object(_)) => {
            Err(DailyRecordError::InvalidField { field: "endTime" })
        }
    }
}

// ROUTE_ENDPOINT_MAPPING.get(call_type, None) behind a truthiness guard: a missing or empty call type maps to no endpoint.
fn call_endpoint(payload: &DailyPayload, endpoints: &dyn CallTypeEndpoints) -> Option<String> {
    let call_type = payload
        .call_type
        .as_deref()
        .filter(|value| !value.is_empty())?;
    endpoints.endpoint(call_type).map(str::to_owned)
}

// _timed_request_duration_ms: only a successful external call carries a timed request, and a bool counts as Python's int does.
fn timed_duration_ms(payload: &DailyPayload, status: RequestStatus, internal: bool) -> Option<i64> {
    if internal || status != RequestStatus::Success {
        return None;
    }
    let duration = payload.request_duration_ms.as_ref()?;
    match duration {
        Scalar::Bool(flag) => Some(i64::from(u8::from(*flag))),
        Scalar::Number(number) => integer_ms(number),
        Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

// isinstance(value, int) against the logged lexeme: a fractional or exponent form is a float, and a negative count is no duration.
fn integer_ms(number: &Number) -> Option<i64> {
    let lexeme = number.to_string();
    if lexeme.contains(['.', 'e', 'E']) {
        return None;
    }
    lexeme.parse::<i64>().ok().filter(|value| *value >= 0)
}

pub(super) fn request_counts(internal: bool, status: RequestStatus) -> (i64, i64, i64) {
    if internal {
        return (0, 0, 0);
    }
    let successful = i64::from(status == RequestStatus::Success);
    let failed = i64::from(status != RequestStatus::Success);
    (1, successful, failed)
}
