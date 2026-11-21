use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::price_lookup::PricedModel;
use crate::savings_evidence::numeric::numeric_savings;
use crate::savings_evidence::pricing_basis::PricingBasis;
use crate::savings_evidence::snapshot::{
    CostBreakdown, pricing_basis_from_breakdown, recorded_token_cost,
};
use crate::savings_evidence::usage::baseline::cache_token_split;
use crate::tiered_pricing::Scalar;

use super::compare::{CostOfUsageInput, amounts_close, cost_of_usage, same_prices, same_usage};
use super::context::{ModelIdentity, SavingsPricingContext, public_price, resolve_model};
use super::usage::{RequestUsage, baseline_request_usage};

pub mod logging;

// compute_autorouter_savings' provenance literal: the baseline was observed on this request or modeled.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoRouterProvenance {
    ObservedInitial,
    Modeled,
}

// The decision keys the netting reads; every other key is ignored.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AutoRouterDecision {
    pub classifier_cost: Option<Scalar>,
    pub savings_baseline_model: Option<Scalar>,
    pub savings_baseline_deployment_id: Option<Scalar>,
}

pub struct AutoRouterSavingsInput<'input> {
    pub baseline_model: Option<&'input str>,
    pub selected_model: Option<&'input str>,
    pub selected_provider: Option<&'input str>,
    pub usage: &'input RequestUsage,
    pub selected_info: Option<&'input PricedModel>,
    pub baseline_info: Option<&'input PricedModel>,
    pub cost_breakdown: Option<&'input CostBreakdown>,
    pub baseline_deployment_id: Option<&'input str>,
    pub selected_deployment_id: Option<&'input str>,
    pub baseline_usage: Option<&'input RequestUsage>,
    pub baseline_provenance: Option<AutoRouterProvenance>,
    pub moment: DateTime<Utc>,
    pub database: &'input TimeZoneDatabase,
}

struct ComparisonValues<'input> {
    baseline: &'input ModelIdentity,
    selected: &'input ModelIdentity,
    baseline_prices: &'input Option<PricedModel>,
    recorded_selected_cost: Option<Decimal>,
    baseline_cost: Decimal,
}

// compute_autorouter_savings: price established baseline usage; conversation shape cannot establish cache warmth.
pub fn compute_autorouter_savings<C>(
    context: &C,
    input: &AutoRouterSavingsInput<'_>,
) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    let baseline = resolve_model(context, input.baseline_model, None)?;
    let selected = resolve_model(context, input.selected_model, input.selected_provider)?;
    let (cache_read, cache_creation) = cache_token_split(&input.usage.usage);
    if input.baseline_usage.is_none() && (cache_read != 0 || cache_creation != 0) {
        return None;
    }
    let prices = input
        .baseline_info
        .cloned()
        .or_else(|| public_price(context, &baseline));
    let basis = pricing_basis_from_breakdown(input.cost_breakdown);
    let baseline_cost = baseline_token_cost(context, input, &baseline, prices.as_ref(), &basis)?;
    let recorded = recorded_token_cost(input.cost_breakdown);
    let selected_cost = match recorded {
        Some(cost) => cost,
        None => cost_of_usage(
            context,
            &CostOfUsageInput {
                identity: &selected,
                usage: input.usage,
                model_info: input.selected_info,
                basis: &basis,
                moment: input.moment,
                database: input.database,
            },
        )?,
    };
    if input.baseline_provenance == Some(AutoRouterProvenance::ObservedInitial) {
        let values = ComparisonValues {
            baseline: &baseline,
            selected: &selected,
            baseline_prices: &prices,
            recorded_selected_cost: recorded,
            baseline_cost,
        };
        return observed_equivalent(context, input, &values).then_some(Decimal::ZERO);
    }
    baseline_cost.checked_sub(selected_cost)
}

fn baseline_token_cost<C>(
    context: &C,
    input: &AutoRouterSavingsInput<'_>,
    baseline: &ModelIdentity,
    prices: Option<&PricedModel>,
    basis: &PricingBasis,
) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    let source = input.baseline_usage.unwrap_or(input.usage);
    let usage = baseline_request_usage(source, prices.map(|stored| &stored.record));
    cost_of_usage(
        context,
        &CostOfUsageInput {
            identity: baseline,
            usage: &usage,
            model_info: prices,
            basis,
            moment: input.moment,
            database: input.database,
        },
    )
}

fn observed_equivalent<C>(
    context: &C,
    input: &AutoRouterSavingsInput<'_>,
    values: &ComparisonValues<'_>,
) -> bool
where
    C: SavingsPricingContext,
{
    let Some(baseline_usage) = input.baseline_usage else {
        return false;
    };
    let selected_prices = input.selected_info.map_or_else(
        || public_price(context, values.selected).map(|supplied| supplied.record),
        |priced| Some(priced.record.clone()),
    );
    let same_deployment = input
        .baseline_deployment_id
        .is_some_and(|id| !id.is_empty())
        && input.baseline_deployment_id == input.selected_deployment_id;
    same_usage(baseline_usage, input.usage)
        && values.baseline == values.selected
        && same_deployment
        && same_prices(
            values.baseline_prices.as_ref().map(|stored| &stored.record),
            selected_prices.as_ref(),
        )
        && values
            .recorded_selected_cost
            .is_some_and(|cost| amounts_close(values.baseline_cost, cost))
}

pub struct AutoRouterRequestInput<'input> {
    pub model: Option<&'input str>,
    pub custom_llm_provider: Option<&'input str>,
    pub routing_decision: Option<&'input AutoRouterDecision>,
    pub usage: Option<&'input RequestUsage>,
    pub model_id: Option<&'input str>,
    pub cost_breakdown: Option<&'input CostBreakdown>,
    pub baseline_usage: Option<&'input RequestUsage>,
    pub baseline_provenance: Option<AutoRouterProvenance>,
    pub moment: DateTime<Utc>,
    pub database: &'input TimeZoneDatabase,
}

// autorouter_savings_for_request: resolve both deployment prices, compute gross and subtract the classifier.
pub async fn autorouter_savings_for_request<C>(
    context: &C,
    input: &AutoRouterRequestInput<'_>,
) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    let usage = input.usage?;
    let model = input.model?;
    let decision = input.routing_decision?;
    let baseline_model = text_value(decision.savings_baseline_model.as_ref())?;
    let baseline_id = text_value(decision.savings_baseline_deployment_id.as_ref());
    let selected_info = context.deployment_price(input.model_id, model).await;
    let baseline_info = match baseline_id {
        Some(id) => context.deployment_price(Some(id), baseline_model).await,
        None => None,
    };
    let gross = compute_autorouter_savings(
        context,
        &AutoRouterSavingsInput {
            baseline_model: Some(baseline_model),
            selected_model: Some(model),
            selected_provider: input.custom_llm_provider,
            usage,
            selected_info: selected_info.as_ref(),
            baseline_info: baseline_info.as_ref(),
            cost_breakdown: input.cost_breakdown,
            baseline_deployment_id: baseline_id,
            selected_deployment_id: input.model_id,
            baseline_usage: input.baseline_usage,
            baseline_provenance: input.baseline_provenance,
            moment: input.moment,
            database: input.database,
        },
    )?;
    let Some(classifier) = numeric_savings(decision.classifier_cost.as_ref()) else {
        return Some(gross);
    };
    gross.checked_sub(classifier)
}

fn text_value(value: Option<&Scalar>) -> Option<&str> {
    match value {
        Some(Scalar::Text(text)) => Some(text),
        Some(
            Scalar::Null
            | Scalar::Bool(_)
            | Scalar::Number(_)
            | Scalar::Array(_)
            | Scalar::Object(_),
        )
        | None => None,
    }
}

mod tests;
