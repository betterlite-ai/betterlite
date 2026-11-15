use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::billed_tokens::TokenTypeCostBreakdown;
use betterlite_models::tool_fees::billing::BuiltInToolRequest;
use rust_decimal::Decimal;

use super::Round;
use super::args::CostCalculator;
use super::breakdown::{BreakdownRequest, token_type_breakdown};
use super::error::CompletionCostError;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::{CallPriceEnv, call_price};
use crate::cost::context::storage::store_cost_breakdown;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{AttemptOutcome, PreparedBillingAttempt};
use crate::cost::special::policy::ModelResolver;
use crate::cost::special::request::{RealtimeSession, SpecialOutcome, TokenStage};
use crate::cost::special::special_charge;
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
    let tool_usage_cost =
        services
            .tools
            .billing()
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

// The token-type breakdown is computed only for a priced usage block with a model name; every other attempt records none.
fn token_breakdown<Resolver, ImagePolicy, Inference>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    attempt: &Attempt<'_>,
    owned: &Owned,
) -> Result<Option<TokenTypeCostBreakdown>, CompletionCostError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
{
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
    Ok(breakdown)
}

// Hands the breakdown to the logging target; a failing setter never replaces a charge that is already known.
fn record_breakdown<Target>(
    sink: &mut Target,
    attempt: &Attempt<'_>,
    breakdown: Option<TokenTypeCostBreakdown>,
    charge: &Charge,
) where
    Target: CostBreakdownTarget,
{
    let inputs = &attempt.args.charge;
    let adjusted = charge.adjusted;
    store_cost_breakdown(
        Some(sink),
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
}

// Ports completion_cost L1811-L1959 for one attempt: price the tokens, add fees, adjust, and record the breakdown.
pub(super) fn attempt_charge<Resolver, ImagePolicy, Inference, Target>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    round: &Round<'_>,
    stage: TokenStage,
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
    let Some(named) = model.as_deref() else {
        return Err(CompletionCostError::Price(CallPriceError::NullModel));
    };
    let attempt = Attempt {
        args: round.args,
        prepared: round.prepared,
        moment: round.moment,
        model: named,
        provider: custom_llm_provider.as_deref(),
        region: region_name.as_deref(),
        request_model_for_cost: stage.request_model_for_cost.as_deref(),
        response_time_ms: stage.response_time_ms,
        billing: &billing,
    };
    let owned = Owned::new::<ImagePolicy::Error>(&attempt, services.special.lookup)?;
    let charge = price_attempt(services, &attempt, &owned)?;
    let final_cost = charge.adjusted.final_cost;
    let Some(sink) = target else {
        return Ok(final_cost);
    };
    let breakdown = token_breakdown(services, &attempt, &owned)?;
    record_breakdown(sink, &attempt, breakdown, &charge);
    Ok(final_cost)
}

// One candidate name: B1689 may finish the charge early, a failed preparation is raised as it is, and anything else goes to the token stage.
pub(super) fn one_attempt<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    round: &Round<'_>,
    attempt: PreparedBillingAttempt,
    clock: &mut Clock,
    mut target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
{
    let realtime = round.session.map(|(results, usage)| RealtimeSession {
        results,
        usage: Some(usage),
    });
    let request =
        round
            .args
            .charge
            .special_request(round.billing, round.prepared, round.moment, realtime);
    let special = services.special.services();
    match special_charge(&special, &request, attempt, clock, target.as_deref_mut())? {
        SpecialOutcome::Charged(charge) => Ok(charge),
        // special_charge hands an attempt back unchanged only when its preparation failed.
        SpecialOutcome::Failed(failed) => match failed.outcome {
            AttemptOutcome::Failed(error) => Err(CompletionCostError::Prepare(error)),
            AttemptOutcome::Ready(_) => Err(CompletionCostError::NoCandidates),
        },
        SpecialOutcome::Continue(stage) => attempt_charge(services, round, *stage, target),
    }
}
