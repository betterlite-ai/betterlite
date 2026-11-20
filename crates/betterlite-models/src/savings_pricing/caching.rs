use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use crate::billed_tokens::BilledTokenRequest;
use crate::billed_tokens::savings::model_caching_savings;
use crate::price_lookup::PricedModel;
use crate::savings_evidence::billed_at::{BilledAt, coerce_billed_at};
use crate::savings_evidence::snapshot::{CostBreakdown, pricing_basis_from_breakdown};

use super::context::{SavingsPricingContext, public_price, resolve_model};
use super::usage::{RequestUsage, billed_usage};

pub struct RequestPricingInput<'input> {
    pub model: Option<&'input str>,
    pub custom_llm_provider: Option<&'input str>,
    pub model_id: Option<&'input str>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestSavingsPricing {
    pub provider: Option<String>,
    pub pricing: Option<PricedModel>,
}

// _request_savings_pricing: a truthy deployment mapping wins, otherwise the canonical public rates.
#[must_use]
pub async fn request_savings_pricing<C>(
    context: &C,
    input: &RequestPricingInput<'_>,
) -> RequestSavingsPricing
where
    C: SavingsPricingContext,
{
    let identity = resolve_model(context, input.model, input.custom_llm_provider);
    let deployment = context
        .deployment_price(input.model_id, input.model.unwrap_or_default())
        .await;
    let pricing = deployment
        .filter(|value| !value.record.is_empty())
        .or_else(|| {
            identity
                .as_ref()
                .and_then(|value| public_price(context, value))
        });
    let provider = identity
        .as_ref()
        .map(|value| value.provider.clone())
        .or_else(|| input.custom_llm_provider.map(str::to_owned));
    RequestSavingsPricing { provider, pricing }
}

pub struct PromptCachingInput<'input> {
    pub pricing: Option<&'input PricedModel>,
    pub provider: Option<&'input str>,
    pub usage: Option<&'input RequestUsage>,
    pub cost_breakdown: Option<&'input CostBreakdown>,
    pub billed_at: &'input BilledAt,
    pub fallback_moment: DateTime<Utc>,
    pub database: &'input TimeZoneDatabase,
}

// _prompt_caching_savings: B1673's net read-discount-minus-write-premium at the recorded basis.
#[must_use]
pub fn prompt_caching_savings(input: &PromptCachingInput<'_>) -> Option<Decimal> {
    let pricing = input.pricing?;
    let usage = input.usage?;
    let basis = pricing_basis_from_breakdown(input.cost_breakdown);
    let price = BilledTokenRequest {
        model: "",
        provider: input.provider,
        pricing: Some(pricing),
        service_tier: basis.service_tier.as_deref(),
        residency: basis.data_residency.as_deref(),
        vertex_location: basis.vertex_location.as_deref(),
        inference_geo: None,
        provider_specific_entry: None,
        moment: billing_moment(input.billed_at, input.fallback_moment),
        prompt_tokens: usage.usage.prompt_tokens,
    };
    model_caching_savings(&price, pricing, billed_usage(usage), input.database).ok()
}

pub struct PromptCachingForRequest<'input> {
    pub model: Option<&'input str>,
    pub custom_llm_provider: Option<&'input str>,
    pub usage: Option<&'input RequestUsage>,
    pub model_id: Option<&'input str>,
    pub cost_breakdown: Option<&'input CostBreakdown>,
    pub billed_at: &'input BilledAt,
    pub fallback_moment: DateTime<Utc>,
    pub database: &'input TimeZoneDatabase,
}

// caching_savings_for_request: the recorded request's own pricing, then the pure calculation.
pub async fn caching_savings_for_request<C>(
    context: &C,
    input: &PromptCachingForRequest<'_>,
) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    let pricing = request_savings_pricing(
        context,
        &RequestPricingInput {
            model: input.model,
            custom_llm_provider: input.custom_llm_provider,
            model_id: input.model_id,
        },
    )
    .await;
    prompt_caching_savings(&PromptCachingInput {
        pricing: pricing.pricing.as_ref(),
        provider: pricing.provider.as_deref(),
        usage: input.usage,
        cost_breakdown: input.cost_breakdown,
        billed_at: input.billed_at,
        fallback_moment: input.fallback_moment,
        database: input.database,
    })
}

// _coerce_billed_at, with the injected request clock standing in for an absent or malformed value.
fn billing_moment(billed_at: &BilledAt, fallback: DateTime<Utc>) -> DateTime<Utc> {
    let owned = match billed_at {
        BilledAt::Missing => BilledAt::Missing,
        BilledAt::Timestamp(timestamp) => BilledAt::Timestamp(*timestamp),
        BilledAt::Text(text) => BilledAt::Text(text.clone()),
    };
    coerce_billed_at(owned).unwrap_or(fallback)
}

mod tests;
