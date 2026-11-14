use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{AdjustmentEntry, AdjustmentKind};
use betterlite_cost::adjustments::{AdjustmentError, apply_cost_adjustment};
use rust_decimal::Decimal;

pub(super) struct Adjusted {
    pub final_cost: Decimal,
    pub discount_percent: Decimal,
    pub discount_amount: Decimal,
    pub margin_percent: Decimal,
    pub margin_fixed_amount: Decimal,
    pub margin_total_amount: Decimal,
}

// Discount first, then margin; a section without entries leaves the cost alone and records zeros.
pub(super) fn adjust(
    total: Decimal,
    provider: Option<&str>,
    discount: &BTreeMap<String, AdjustmentEntry>,
    margin: &BTreeMap<String, AdjustmentEntry>,
) -> Result<Adjusted, AdjustmentError> {
    let mut adjusted = Adjusted {
        final_cost: total,
        discount_percent: Decimal::ZERO,
        discount_amount: Decimal::ZERO,
        margin_percent: Decimal::ZERO,
        margin_fixed_amount: Decimal::ZERO,
        margin_total_amount: Decimal::ZERO,
    };
    if !discount.is_empty() {
        let result = apply_cost_adjustment(
            AdjustmentKind::Discount,
            adjusted.final_cost,
            provider,
            discount,
        )?;
        adjusted.final_cost = result.final_cost;
        adjusted.discount_percent = result.percent;
        adjusted.discount_amount = result.amount;
    }
    if !margin.is_empty() {
        let result = apply_cost_adjustment(
            AdjustmentKind::Margin,
            adjusted.final_cost,
            provider,
            margin,
        )?;
        adjusted.final_cost = result.final_cost;
        adjusted.margin_percent = result.percent;
        adjusted.margin_fixed_amount = result.fixed_amount;
        adjusted.margin_total_amount = result.amount;
    }
    Ok(adjusted)
}
