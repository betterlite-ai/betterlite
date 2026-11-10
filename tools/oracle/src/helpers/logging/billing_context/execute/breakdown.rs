use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::billed_tokens::BilledTokenRates;
use serde_json::{Value, json};

use super::super::model::{breakdown_input, logging_state};
use super::super::projection::{optional_costs, optional_decimal, optional_text};
use super::super::{Call, Failure, failure};

pub(super) fn breakdown(call: &Call) -> Result<Value, Failure> {
    let mut state = logging_state()?;
    let mut records = Vec::with_capacity(call.breakdowns.len());
    for input in &call.breakdowns {
        let converted = breakdown_input(input);
        CostBreakdownTarget::set_cost_breakdown(&mut state, &converted).map_err(failure)?;
        records.push(record_value(&state));
    }
    Ok(Value::Array(records))
}

fn record_value(state: &LoggingState) -> Value {
    json!({
        "cost_breakdown": state.cost_breakdown.as_ref().map_or(Value::Null, |record| json!({
            "input_cost": optional_decimal(record.input_cost),
            "output_cost": optional_decimal(record.output_cost),
            "total_cost": optional_decimal(record.total_cost),
            "tool_usage_cost": optional_decimal(record.tool_usage_cost),
            "cache_read_cost": optional_decimal(record.cache_read_cost),
            "cache_creation_cost": optional_decimal(record.cache_creation_cost),
            "reasoning_cost": optional_decimal(record.reasoning_cost),
            "additional_costs": optional_costs(record.additional_costs.as_ref()),
            "original_cost": optional_decimal(record.original_cost),
            "discount_percent": optional_decimal(record.discount_percent),
            "discount_amount": optional_decimal(record.discount_amount),
            "margin_percent": optional_decimal(record.margin_percent),
            "margin_fixed_amount": optional_decimal(record.margin_fixed_amount),
            "margin_total_amount": optional_decimal(record.margin_total_amount),
            "service_tier": optional_text(record.service_tier.as_deref()),
            "data_residency": optional_text(record.data_residency.as_deref()),
            "vertex_location": optional_text(record.vertex_location.as_deref()),
        })),
        "billed_token_rates": state.billed_token_rates.map_or(Value::Null, rates_value),
    })
}

fn rates_value(rates: BilledTokenRates) -> Value {
    json!({
        "input_cost_per_token": rates.input_cost_per_token.normalize().to_string(),
        "output_cost_per_token": rates.output_cost_per_token.normalize().to_string(),
        "cache_read_input_token_cost": rates.cache_read_input_token_cost.normalize().to_string(),
        "audio_read": rates.audio_read.normalize().to_string(),
        "cache_write": rates.cache_write.normalize().to_string(),
        "cache_write_1h": rates.cache_write_1h.normalize().to_string(),
        "reasoning": rates.reasoning.normalize().to_string(),
    })
}
