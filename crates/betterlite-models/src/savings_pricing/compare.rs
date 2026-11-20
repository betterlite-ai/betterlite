use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;

use crate::anthropic_pricing::{
    ANTHROPIC_PROVIDER, AnthropicPriceRequest, TokenPriceRequest, token_cost,
};
use crate::cost_usage::server_tool_projection::ServerToolProjection;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::PricedModel;
use crate::savings_evidence::error::SavingsEvidenceError;
use crate::savings_evidence::numeric::{RoutingDecision, numeric_savings};
use crate::savings_evidence::pricing_basis::PricingBasis;
use crate::savings_evidence::snapshot::{
    BaselineCostSnapshot, CostBreakdown, pricing_basis_from_breakdown, recorded_token_cost,
};
use crate::savings_evidence::spend::BaselineCosts;
use crate::savings_evidence::usage::{LoggedUsage, PromptTokensDetails};
use crate::tiered_pricing::Scalar;
use crate::token_cost::{GenericCostRequest, generic_cost_per_token};
use crate::token_rates::ModelInfo;

use super::context::{ModelIdentity, SavingsPricingContext};
use super::usage::{RequestUsage, cost_usage, prompt_extras};

// price_baseline_comparison's provenance literal: the baseline was observed on the same deployment or modeled.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonProvenance {
    ObservedIdentical,
    Modeled,
}

pub struct CostOfUsageInput<'input> {
    pub identity: &'input ModelIdentity,
    pub usage: &'input RequestUsage,
    pub model_info: Option<&'input PricedModel>,
    pub basis: &'input PricingBasis,
    pub moment: DateTime<Utc>,
    pub database: &'input TimeZoneDatabase,
}

// _cost_of_usage: anthropic prices with tier only, every other provider with the full basis.
pub fn cost_of_usage<C>(context: &C, request: &CostOfUsageInput<'_>) -> Option<Decimal>
where
    C: SavingsPricingContext,
{
    let usage = cost_usage(request.usage);
    let extras = prompt_extras(request.usage);
    let projection = ServerToolProjection::default();
    let costs = if request.identity.provider == ANTHROPIC_PROVIDER {
        let price = TokenPriceRequest::Anthropic(AnthropicPriceRequest {
            model: &request.identity.model,
            usage: &usage,
            prompt_extras: &extras,
            supplied_model_info: request.model_info.map(|priced| &priced.record),
            provider_specific_entry: None,
            service_tier: request.basis.service_tier.as_deref(),
            projection: &projection,
            moment: request.moment,
        });
        token_cost(&price, context.public_lookup(), request.database).ok()?
    } else {
        let price = GenericCostRequest {
            model: &request.identity.model,
            custom_llm_provider: Some(&request.identity.provider),
            usage: &usage,
            prompt_extras: &extras,
            supplied_pricing: request.model_info,
            service_tier: request.basis.service_tier.as_deref(),
            data_residency: request.basis.data_residency.as_deref(),
            vertex_location: request.basis.vertex_location.as_deref(),
            moment: request.moment,
        };
        generic_cost_per_token(&price, context.public_lookup(), request.database).ok()?
    };
    costs.prompt_cost.checked_add(costs.completion_cost)
}

pub struct SnapshotInput<'input> {
    pub model: &'input str,
    pub prices: Option<&'input ModelInfo>,
    pub actual_spend: Decimal,
    pub cost_breakdown: Option<&'input CostBreakdown>,
    pub routing_decision: Option<&'input RoutingDecision>,
}

// baseline_cost_snapshot: the recorded evidence a later comparison reads, through B1693's strict snapshot.
pub fn baseline_cost_snapshot(
    request: &SnapshotInput<'_>,
) -> Result<BaselineCostSnapshot, SavingsEvidenceError> {
    let actual_spend = checked_amount(request.actual_spend, "actual_spend")?;
    let actual_token_cost = recorded_token_cost(request.cost_breakdown)
        .map(|cost| checked_amount(cost, "actual_token_cost"))
        .transpose()?;
    let classifier_cost = numeric_savings(
        request
            .routing_decision
            .and_then(|decision| decision.classifier_cost.as_ref()),
    )
    .map(|cost| checked_amount(cost, "classifier_cost"))
    .transpose()?
    .unwrap_or(Decimal::ZERO);
    Ok(BaselineCostSnapshot {
        model: request.model.to_owned(),
        provider: ANTHROPIC_PROVIDER.to_owned(),
        prices: request.prices.cloned(),
        basis: pricing_basis_from_breakdown(request.cost_breakdown),
        actual_spend,
        actual_token_cost,
        classifier_cost,
    })
}

const fn checked_amount(
    amount: Decimal,
    field: &'static str,
) -> Result<Decimal, SavingsEvidenceError> {
    if amount.is_sign_negative() {
        return Err(SavingsEvidenceError::NegativeAmount { field });
    }
    Ok(amount)
}

pub struct ComparisonInput<'input> {
    pub snapshot: &'input BaselineCostSnapshot,
    pub baseline_usage: Option<&'input RequestUsage>,
    pub provenance: Option<ComparisonProvenance>,
    pub moment: DateTime<Utc>,
    pub database: &'input TimeZoneDatabase,
}

// price_baseline_comparison: replace only the baseline token portion, keeping every non-token charge on both sides.
pub fn price_baseline_comparison<C>(
    context: &C,
    request: &ComparisonInput<'_>,
) -> Option<BaselineCosts>
where
    C: SavingsPricingContext,
{
    let usage = request.baseline_usage?;
    let provenance = request.provenance?;
    let actual = request
        .snapshot
        .actual_spend
        .checked_add(request.snapshot.classifier_cost)?;
    if provenance == ComparisonProvenance::ObservedIdentical {
        return Some(BaselineCosts {
            actual,
            baseline: request.snapshot.actual_spend,
        });
    }
    modeled_baseline(context, request, usage, actual)
}

// The baseline tokens at the snapshot's own recorded rates, as a supplied model info table.
fn modeled_baseline<C>(
    context: &C,
    request: &ComparisonInput<'_>,
    usage: &RequestUsage,
    actual: Decimal,
) -> Option<BaselineCosts>
where
    C: SavingsPricingContext,
{
    let prices = request.snapshot.prices.as_ref()?;
    let recorded = request.snapshot.actual_token_cost?;
    let token_cost = cost_of_usage(
        context,
        &CostOfUsageInput {
            identity: &ModelIdentity {
                model: request.snapshot.model.clone(),
                provider: request.snapshot.provider.clone(),
            },
            usage,
            model_info: Some(&PricedModel {
                record: prices.clone(),
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            }),
            basis: &request.snapshot.basis,
            moment: request.moment,
            database: request.database,
        },
    )?;
    if token_cost < Decimal::ZERO {
        return None;
    }
    let baseline = request
        .snapshot
        .actual_spend
        .checked_add(token_cost)?
        .checked_sub(recorded)?;
    if baseline < Decimal::ZERO {
        return None;
    }
    Some(BaselineCosts { actual, baseline })
}

// effective_baseline_info == selected_model_info, with nested values conservatively unequal.
#[must_use]
pub fn same_prices(left: Option<&ModelInfo>, right: Option<&ModelInfo>) -> bool {
    if left.is_none() && right.is_none() {
        return true;
    }
    let (Some(first), Some(second)) = (left, right) else {
        return false;
    };
    first.len() == second.len()
        && first.iter().all(|(key, value)| {
            second
                .get(key)
                .is_some_and(|other| same_scalar(value, other))
        })
}

fn same_scalar(left: &Scalar, right: &Scalar) -> bool {
    match left {
        Scalar::Null => matches!(right, Scalar::Null),
        Scalar::Bool(flag) => matches!(right, Scalar::Bool(other) if other == flag),
        Scalar::Text(text) => matches!(right, Scalar::Text(other) if other == text),
        Scalar::Number(value) => {
            matches!(right, Scalar::Number(other) if same_number(value, other))
        }
        Scalar::Array(_) | Scalar::Object(_) => false,
    }
}

fn same_number(left: &Number, right: &Number) -> bool {
    let left_lexeme = left.to_string();
    let right_lexeme = right.to_string();
    match (
        parse_decimal_lexeme(&left_lexeme),
        parse_decimal_lexeme(&right_lexeme),
    ) {
        (Ok(left_value), Ok(right_value)) => left_value == right_value,
        _ => left_lexeme == right_lexeme,
    }
}

// baseline_usage == usage, field by field over the parsed usage and the TTL split.
#[must_use]
pub fn same_usage(left: &RequestUsage, right: &RequestUsage) -> bool {
    same_logged_usage(&left.usage, &right.usage)
        && left.cache_creation_token_details == right.cache_creation_token_details
}

fn same_logged_usage(left: &LoggedUsage, right: &LoggedUsage) -> bool {
    let counts_match = left.prompt_tokens == right.prompt_tokens
        && left.completion_tokens == right.completion_tokens
        && left.total_tokens == right.total_tokens
        && left.cache_read_input_tokens == right.cache_read_input_tokens
        && left.cache_creation_input_tokens == right.cache_creation_input_tokens;
    counts_match
        && left.completion_tokens_details == right.completion_tokens_details
        && same_prompt_details(
            left.prompt_tokens_details.as_ref(),
            right.prompt_tokens_details.as_ref(),
        )
}

// The prompt counters compared one by one; the TTL split B1693 keeps opaque is compared by same_usage.
const fn same_prompt_details(
    left: Option<&PromptTokensDetails>,
    right: Option<&PromptTokensDetails>,
) -> bool {
    match (left, right) {
        (Some(baseline), Some(selected)) => same_prompt_counters(baseline, selected),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

const fn same_prompt_counters(
    baseline: &PromptTokensDetails,
    selected: &PromptTokensDetails,
) -> bool {
    baseline.cached_tokens == selected.cached_tokens
        && baseline.cache_creation_tokens == selected.cache_creation_tokens
        && baseline.cache_write_tokens == selected.cache_write_tokens
        && baseline.text_tokens == selected.text_tokens
        && baseline.audio_tokens == selected.audio_tokens
        && baseline.image_tokens == selected.image_tokens
        && baseline.video_tokens == selected.video_tokens
}

// math.isclose with the source's rel_tol=1e-9 and abs_tol=1e-12, in Decimal.
#[must_use]
pub fn amounts_close(left: Decimal, right: Decimal) -> bool {
    let Some(signed_difference) = left.checked_sub(right) else {
        return false;
    };
    let difference = signed_difference.abs();
    let scale = left.abs().max(right.abs());
    let Some(relative) = scale.checked_mul(Decimal::new(1, 9)) else {
        return false;
    };
    difference <= relative.max(Decimal::new(1, 12))
}

mod tests;
