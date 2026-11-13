use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::billed_tokens::BilledTokenRates;
use serde_json::{Value, json};

use super::super::model::logging_state;
use super::super::projection::{optional_costs, optional_decimal};
use super::super::{Call, Failure, failure};

pub(super) fn breakdown(call: &Call) -> Result<Value, Failure> {
    let mut state = logging_state()?;
    let mut records = Vec::with_capacity(call.breakdowns.len());
    for input in &call.breakdowns {
        let converted = CostBreakdownInput {
            input_cost: input.input_cost,
            output_cost: input.output_cost,
            total_cost: input.total_cost,
            tool_usage_cost: input.tool_usage_cost,
            additional_costs: input.additional_costs.clone(),
            original_cost: input.original_cost,
            discount_percent: input.discount_percent,
            discount_amount: input.discount_amount,
            margin_percent: input.margin_percent,
            margin_fixed_amount: input.margin_fixed_amount,
            margin_total_amount: input.margin_total_amount,
            cache_read_cost: input.cache_read_cost,
            cache_creation_cost: input.cache_creation_cost,
            reasoning_cost: input.reasoning_cost,
            service_tier: input.service_tier.clone(),
            data_residency: input.data_residency.clone(),
            vertex_location: input.vertex_location.clone(),
            billed_token_rates: input.billed_token_rates.map(|rates| BilledTokenRates {
                input_cost_per_token: rates.input_cost_per_token,
                output_cost_per_token: rates.output_cost_per_token,
                cache_read_input_token_cost: rates.cache_read_input_token_cost,
                audio_read: rates.audio_read,
                cache_write: rates.cache_write,
                cache_write_1h: rates.cache_write_1h,
                reasoning: rates.reasoning,
            }),
        };
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
            "service_tier": record.service_tier,
            "data_residency": record.data_residency,
            "vertex_location": record.vertex_location,
        })),
        "billed_token_rates": state.billed_token_rates.map_or(Value::Null, |rates| json!({
            "input_cost_per_token": rates.input_cost_per_token.normalize().to_string(),
            "output_cost_per_token": rates.output_cost_per_token.normalize().to_string(),
            "cache_read_input_token_cost": rates.cache_read_input_token_cost.normalize().to_string(),
            "audio_read": rates.audio_read.normalize().to_string(),
            "cache_write": rates.cache_write.normalize().to_string(),
            "cache_write_1h": rates.cache_write_1h.normalize().to_string(),
            "reasoning": rates.reasoning.normalize().to_string(),
        })),
    })
}
