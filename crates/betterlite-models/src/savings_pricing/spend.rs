use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use crate::modality_cost::pricing::price_for_key;
use crate::price_lookup::PricedModel;
use crate::savings_evidence::billed_at::BilledAt;
use crate::savings_evidence::dynamic::MappingOrScalar;
use crate::savings_evidence::metadata::{AutorouterEstimate, Metadata};
use crate::savings_evidence::numeric::{numeric_savings, recorded_autorouter_savings};
use crate::savings_evidence::snapshot::CostBreakdown;
use crate::savings_evidence::spend::SavingsSpend;
use crate::tiered_pricing::Scalar;

use super::autorouter::{
    AutoRouterDecision, AutoRouterRequestInput, autorouter_savings_for_request,
};
use super::caching::{
    PromptCachingInput, RequestPricingInput, prompt_caching_savings, request_savings_pricing,
};
use super::context::SavingsPricingContext;
use super::usage::RequestUsage;

pub struct SavingsSpendInput<'a> {
    pub model: Option<&'a str>,
    pub custom_llm_provider: Option<&'a str>,
    pub compression_saved_tokens: i64,
    pub gateway_injected_cache: bool,
    pub routing_decision: Option<&'a AutoRouterDecision>,
    pub usage: Option<&'a RequestUsage>,
    pub model_id: Option<&'a str>,
    pub cost_breakdown: Option<&'a CostBreakdown>,
    pub recorded_autorouter_savings: Option<&'a Scalar>,
    pub recorded_autorouter_savings_estimate: Option<&'a MappingOrScalar<AutorouterEstimate>>,
    pub billed_at: &'a BilledAt,
    pub fallback_moment: DateTime<Utc>,
    pub database: &'a TimeZoneDatabase,
}

// compute_savings_spend: every driver priced on its own rates, recorded values honored before recomputation.
pub async fn compute_savings_spend<C>(context: &C, input: &SavingsSpendInput<'_>) -> SavingsSpend
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
    let compression = compression_savings(input, pricing.pricing.as_ref());
    let prompt_caching = prompt_caching_savings(&PromptCachingInput {
        pricing: pricing.pricing.as_ref(),
        provider: pricing.provider.as_deref(),
        usage: input.usage,
        cost_breakdown: input.cost_breakdown,
        billed_at: input.billed_at,
        fallback_moment: input.fallback_moment,
        database: input.database,
    })
    .unwrap_or(Decimal::ZERO);
    let gateway_injected_caching = if input.gateway_injected_cache {
        prompt_caching
    } else {
        Decimal::ZERO
    };
    let autorouter = match recorded_savings(input) {
        Some(recorded) => recorded,
        None if input.recorded_autorouter_savings_estimate.is_some() => Decimal::ZERO,
        None => request_savings(context, input)
            .await
            .unwrap_or(Decimal::ZERO),
    };
    SavingsSpend {
        compression,
        prompt_caching,
        autorouter,
        gateway_injected_caching,
    }
}

// A nonnull estimate is authoritative even when it holds no usable amount.
fn recorded_savings(input: &SavingsSpendInput<'_>) -> Option<Decimal> {
    let estimate = input.recorded_autorouter_savings_estimate;
    let Some(estimate) = estimate else {
        return numeric_savings(input.recorded_autorouter_savings);
    };
    let metadata = Metadata {
        autorouter_savings_estimate: Some(estimate.clone()),
        autorouter_savings: input.recorded_autorouter_savings.cloned(),
        ..Metadata::default()
    };
    recorded_autorouter_savings(&metadata)
}

async fn request_savings<C>(context: &C, input: &SavingsSpendInput<'_>) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    autorouter_savings_for_request(
        context,
        &AutoRouterRequestInput {
            model: input.model,
            custom_llm_provider: input.custom_llm_provider,
            routing_decision: input.routing_decision,
            usage: input.usage,
            model_id: input.model_id,
            cost_breakdown: input.cost_breakdown,
            baseline_usage: None,
            baseline_provenance: None,
            moment: input.fallback_moment,
            database: input.database,
        },
    )
    .await
}

// max(tokens, 0) at the flat input rate; the caching and autorouter counterfactuals are separate.
fn compression_savings(input: &SavingsSpendInput<'_>, pricing: Option<&PricedModel>) -> Decimal {
    let Some(priced) = pricing else {
        return Decimal::ZERO;
    };
    let rate = price_for_key(&priced.record, "input_cost_per_token")
        .ok()
        .flatten()
        .unwrap_or(Decimal::ZERO);
    Decimal::from(input.compression_saved_tokens.max(0))
        .checked_mul(rate)
        .unwrap_or(Decimal::ZERO)
}

mod tests;
