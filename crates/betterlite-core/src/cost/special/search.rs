use betterlite_config::cost_adjustments::AdjustmentKind;
use betterlite_cost::adjustments::{AdjustmentResult, apply_cost_adjustment};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_models::decisions::Json;
use betterlite_models::token_cost::GenericCostResult;
use rust_decimal::Decimal;

use super::early::Identity;
use super::error::SpecialChargeError;
use super::policy::SearchQuery;
use super::request::{SpecialRequest, SpecialServices};
use crate::cost::context::storage::store_cost_breakdown;
use crate::cost::image_dispatch::policy::ImagePricePolicy;

// A list query bills one search per entry, an empty list included; any other query, or none, bills one.
fn query_count(request: &SpecialRequest<'_>) -> i64 {
    match request
        .optional_params
        .and_then(|params| params.get("query"))
    {
        Some(Json::List(queries)) => i64::try_from(queries.len()).unwrap_or(i64::MAX),
        _ => 1,
    }
}

pub(super) fn search_charge<Resolver, ImagePolicy, Target>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    identity: Identity<'_>,
    target: Option<&mut Target>,
) -> Result<Decimal, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
{
    let Identity { model, provider } = identity;
    // A provider with a slash-free model like "tavily-search" is looked up as "tavily/search".
    let constructed = match provider {
        Some(name) if !name.is_empty() && !model.contains('/') => Some(format!("{name}/search")),
        _ => None,
    };
    let priced = services.special_policy.search_cost(&SearchQuery {
        model: constructed.as_deref().unwrap_or(model),
        provider,
        number_of_queries: query_count(request),
        optional_params: request.optional_params,
    })?;
    let original = priced
        .prompt_cost
        .checked_add(priced.completion_cost)
        .ok_or(SpecialChargeError::Overflow)?;
    let discount = apply_cost_adjustment(
        AdjustmentKind::Discount,
        original,
        provider,
        services.discount,
    )?;
    let margin = apply_cost_adjustment(
        AdjustmentKind::Margin,
        discount.final_cost,
        provider,
        services.margin,
    )?;
    store_cost_breakdown(
        target,
        &search_breakdown(request, &priced, original, (discount, margin)),
    );
    Ok(margin.final_cost)
}

fn search_breakdown(
    request: &SpecialRequest<'_>,
    priced: &GenericCostResult,
    original: Decimal,
    adjustments: (AdjustmentResult, AdjustmentResult),
) -> CostBreakdownInput {
    let (discount, margin) = adjustments;
    CostBreakdownInput {
        input_cost: priced.prompt_cost,
        output_cost: priced.completion_cost,
        total_cost: margin.final_cost,
        tool_usage_cost: Decimal::ZERO,
        additional_costs: None,
        original_cost: Some(original),
        discount_percent: Some(discount.percent),
        discount_amount: Some(discount.amount),
        margin_percent: Some(margin.percent),
        margin_fixed_amount: Some(margin.fixed_amount),
        margin_total_amount: Some(margin.amount),
        cache_read_cost: None,
        cache_creation_cost: None,
        reasoning_cost: None,
        service_tier: request.service_tier.map(str::to_owned),
        data_residency: request.data_residency.map(str::to_owned),
        vertex_location: None,
        billed_token_rates: None,
    }
}
