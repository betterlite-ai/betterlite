use std::collections::BTreeMap;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::tool_fees::billing::BuiltInToolRequest;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::args::{CompletionArgs, CostCalculator};
use super::breakdown::{BreakdownRequest, token_type_breakdown};
use super::error::CompletionCostError;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::{CallPriceEnv, call_price};
use crate::cost::context::storage::store_cost_breakdown;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{AttemptOutcome, PreparedBilling, PreparedBillingAttempt};
use crate::cost::special::request::TokenStage;
use adjust::{Adjusted, adjust};
use extras::{additional_fees, checked_total};
use inputs::{Attempt, Owned, price_input};

mod adjust;
mod extras;
mod inputs;

// What one attempt charged before and after the adjustments, with the parts the breakdown records.
struct Charge {
    input_cost: Decimal,
    output_cost: Decimal,
    tool_usage_cost: Decimal,
    additional_costs: Option<BTreeMap<String, Decimal>>,
    original_cost: Decimal,
    adjusted: Adjusted,
}

// Prices the tokens, adds the built-in tool fees and the provider's extra fees, then applies the discount and the margin.
fn price_attempt<Resolver, ImagePolicy, Inference>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    attempt: &Attempt<'_>,
    owned: &Owned,
) -> Result<Charge, CompletionCostError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
{
    let lookup = services.special.lookup;
    let env = CallPriceEnv {
        lookup,
        policy: services.price_policy,
        resolution: services.resolution,
        database: services.special.database,
        moment: attempt.moment,
    };
    let priced = call_price(&price_input(attempt, owned), &env)?;
    let additional_costs = additional_fees(attempt, lookup)?;
    let params = &attempt.args.charge.tools;
    let tool_usage_cost = services
        .tools
        .cost_for_built_in_tools(&BuiltInToolRequest {
            model: attempt.model,
            custom_llm_provider: attempt.provider,
            response: &params.response,
            google_maps_requests: params.google_maps_requests,
            params: &params.params,
        })?;
    let original_cost = checked_total(
        priced.prompt_cost,
        priced.completion_cost,
        tool_usage_cost,
        additional_costs.as_ref(),
    )?;
    let adjusted = adjust(
        original_cost,
        attempt.provider,
        services.special.discount,
        services.special.margin,
    )?;
    Ok(Charge {
        input_cost: priced.prompt_cost,
        output_cost: priced.completion_cost,
        tool_usage_cost,
        additional_costs,
        original_cost,
        adjusted,
    })
}

// Hands the breakdown to the logging target; a failing setter never replaces the charge, but a failing token-type computation fails the attempt.
fn record_breakdown<Resolver, ImagePolicy, Inference, Target>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    attempt: &Attempt<'_>,
    owned: &Owned,
    charge: Charge,
    target: Option<&mut Target>,
) -> Result<(), CompletionCostError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
{
    let Some(target) = target else {
        return Ok(());
    };
    let breakdown = if attempt.prepared.cost_usage.is_some() && !attempt.model.is_empty() {
        let request = BreakdownRequest {
            model: attempt.model,
            provider: attempt.provider,
            moment: attempt.moment,
        };
        Some(token_type_breakdown(
            &request,
            &price_input(attempt, owned),
            services.special.lookup,
            services.special.database,
        )?)
    } else {
        None
    };
    let inputs = &attempt.args.charge;
    let adjusted = charge.adjusted;
    store_cost_breakdown(
        Some(target),
        &CostBreakdownInput {
            input_cost: charge.input_cost,
            output_cost: charge.output_cost,
            total_cost: adjusted.final_cost,
            tool_usage_cost: charge.tool_usage_cost,
            additional_costs: charge.additional_costs,
            original_cost: Some(charge.original_cost),
            discount_percent: Some(adjusted.discount_percent),
            discount_amount: Some(adjusted.discount_amount),
            margin_percent: Some(adjusted.margin_percent),
            margin_fixed_amount: Some(adjusted.margin_fixed_amount),
            margin_total_amount: Some(adjusted.margin_total_amount),
            cache_read_cost: breakdown.map(|found| found.cache_read_cost),
            cache_creation_cost: breakdown.map(|found| found.cache_creation_cost),
            reasoning_cost: breakdown.map(|found| found.reasoning_cost),
            service_tier: attempt.prepared.service_tier.clone(),
            data_residency: inputs.data_residency.map(str::to_owned),
            vertex_location: inputs.vertex_location.map(str::to_owned),
            billed_token_rates: breakdown.and_then(|found| found.rates),
        },
    );
    Ok(())
}

// Ports completion_cost L1811-L1959 for one attempt: price the tokens, add fees, adjust, and record the breakdown.
pub(super) fn attempt_charge<Resolver, ImagePolicy, Inference, Target>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    args: &CompletionArgs<'_>,
    prepared: &PreparedBilling,
    stage: TokenStage,
    moment: DateTime<Utc>,
    target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
{
    let PreparedBillingAttempt {
        model,
        custom_llm_provider,
        region_name,
        outcome,
    } = stage.attempt;
    let billing = match outcome {
        AttemptOutcome::Ready(billing) => billing,
        AttemptOutcome::Failed(error) => return Err(CompletionCostError::Prepare(error)),
    };
    let Some(model) = model.as_deref() else {
        return Err(CompletionCostError::Price(CallPriceError::NullModel));
    };
    let attempt = Attempt {
        args,
        prepared,
        moment,
        model,
        provider: custom_llm_provider.as_deref(),
        region: region_name.as_deref(),
        request_model_for_cost: stage.request_model_for_cost.as_deref(),
        response_time_ms: stage.response_time_ms,
        billing: &billing,
    };
    let owned = Owned::new::<ImagePolicy::Error>(&attempt, services.special.lookup)?;
    let charge = price_attempt(services, &attempt, &owned)?;
    let final_cost = charge.adjusted.final_cost;
    record_breakdown(services, &attempt, &owned, charge, target)?;
    Ok(final_cost)
}
