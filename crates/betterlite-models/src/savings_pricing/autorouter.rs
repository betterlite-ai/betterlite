use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::price_lookup::PricedModel;
use crate::savings_evidence::numeric::numeric_savings;
use crate::savings_evidence::snapshot::{
    CostBreakdown, pricing_basis_from_breakdown, recorded_token_cost,
};
use crate::savings_evidence::usage::LoggedUsage;
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

pub struct AutoRouterSavingsInput<'a> {
    pub baseline_model: Option<&'a str>,
    pub selected_model: Option<&'a str>,
    pub selected_provider: Option<&'a str>,
    pub usage: &'a RequestUsage,
    pub selected_info: Option<&'a PricedModel>,
    pub baseline_info: Option<&'a PricedModel>,
    pub cost_breakdown: Option<&'a CostBreakdown>,
    pub baseline_deployment_id: Option<&'a str>,
    pub selected_deployment_id: Option<&'a str>,
    pub baseline_usage: Option<&'a RequestUsage>,
    pub baseline_provenance: Option<AutoRouterProvenance>,
    pub moment: DateTime<Utc>,
    pub database: &'a TimeZoneDatabase,
}

struct ComparisonValues<'a> {
    baseline: &'a ModelIdentity,
    selected: &'a ModelIdentity,
    baseline_prices: &'a Option<PricedModel>,
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
    if input.baseline_usage.is_none() && has_cache_buckets(&input.usage.usage) {
        return None;
    }
    let baseline_prices = match input.baseline_info {
        Some(priced) => Some(priced.clone()),
        None => public_price(context, &baseline),
    };
    let source = input.baseline_usage.unwrap_or(input.usage);
    let baseline_usage = baseline_request_usage(
        source,
        baseline_prices.as_ref().map(|priced| &priced.record),
    );
    let basis = pricing_basis_from_breakdown(input.cost_breakdown);
    let baseline_cost = cost_of_usage(
        context,
        &CostOfUsageInput {
            identity: &baseline,
            usage: &baseline_usage,
            model_info: baseline_prices.as_ref(),
            basis: &basis,
            moment: input.moment,
            database: input.database,
        },
    )?;
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
            baseline_prices: &baseline_prices,
            recorded_selected_cost: recorded,
            baseline_cost,
        };
        return observed_equivalent(context, input, &values).then_some(Decimal::ZERO);
    }
    baseline_cost.checked_sub(selected_cost)
}

fn has_cache_buckets(usage: &LoggedUsage) -> bool {
    let (cache_read, cache_creation) = cache_token_split(usage);
    cache_read != 0 || cache_creation != 0
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
    let selected_prices = match input.selected_info {
        Some(priced) => Some(priced.record.clone()),
        None => public_price(context, values.selected).map(|priced| priced.record),
    };
    let same_deployment = input
        .baseline_deployment_id
        .is_some_and(|id| !id.is_empty())
        && input.baseline_deployment_id == input.selected_deployment_id;
    same_usage(baseline_usage, input.usage)
        && values.baseline == values.selected
        && same_deployment
        && same_prices(
            values.baseline_prices.as_ref().map(|priced| &priced.record),
            selected_prices.as_ref(),
        )
        && values
            .recorded_selected_cost
            .is_some_and(|cost| amounts_close(values.baseline_cost, cost))
}

pub struct AutoRouterRequestInput<'a> {
    pub model: Option<&'a str>,
    pub custom_llm_provider: Option<&'a str>,
    pub routing_decision: Option<&'a AutoRouterDecision>,
    pub usage: Option<&'a RequestUsage>,
    pub model_id: Option<&'a str>,
    pub cost_breakdown: Option<&'a CostBreakdown>,
    pub baseline_usage: Option<&'a RequestUsage>,
    pub baseline_provenance: Option<AutoRouterProvenance>,
    pub moment: DateTime<Utc>,
    pub database: &'a TimeZoneDatabase,
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
    let model = input.model.filter(|value| !value.is_empty())?;
    let decision = input.routing_decision?;
    let baseline_model =
        text_value(decision.savings_baseline_model.as_ref()).filter(|value| !value.is_empty())?;
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
    match numeric_savings(decision.classifier_cost.as_ref()) {
        Some(cost) => gross.checked_sub(cost),
        None => Some(gross),
    }
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
