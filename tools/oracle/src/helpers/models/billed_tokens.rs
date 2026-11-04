use betterlite_models::billed_tokens::custom::{
    custom_billed_token_rates, custom_breakdown, custom_cost,
};
use betterlite_models::billed_tokens::{
    BilledTokenError, BilledTokenRates, TokenTypeCostBreakdown, breakdown,
    project_billed_token_counts,
};
use serde_json::{Value, json};

mod catalog;
mod input;
use input::{Input, Operation};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input.operation {
            Operation::Counts => Ok(json!(project_billed_token_counts(input.usage))),
            Operation::CustomCost | Operation::CustomBreakdown => run_custom(input),
            Operation::Catalog | Operation::Savings => catalog::run(input),
        };
        result.map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "BilledTokenError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn run_custom(input: &Input) -> Result<Value, BilledTokenError> {
    let prices = input.custom_cost_per_token.as_ref().map(|rates| {
        (
            rates.input_cost_per_token,
            rates.output_cost_per_token,
            rates.cache_read_input_token_cost,
            rates.cache_write,
        )
    });
    if matches!(input.operation, Operation::CustomCost) {
        return custom_cost(
            input.totals,
            prices,
            input.custom_cost_per_second,
            input.response_time_ms,
        )
        .map(|cost| {
            cost.map_or(Value::Null, |(prompt, completion)| {
                json!({
                    "input_cost": prompt.normalize().to_string(),
                    "output_cost": completion.normalize().to_string(),
                })
            })
        });
    }
    let counts = project_billed_token_counts(input.usage);
    let result = match prices {
        Some((prompt, completion, read, write)) => custom_breakdown(
            counts,
            custom_billed_token_rates(prompt, completion, read, write),
        ),
        None => breakdown(counts, None),
    };
    result.map(breakdown_value)
}

fn breakdown_value(result: TokenTypeCostBreakdown) -> Value {
    json!({
        "reasoning_cost": result.reasoning_cost.normalize().to_string(),
        "cache_read_cost": result.cache_read_cost.normalize().to_string(),
        "cache_creation_cost": result.cache_creation_cost.normalize().to_string(),
        "rates": result.rates.map(rates_value),
    })
}

fn rates_value(rates: BilledTokenRates) -> Value {
    json!({
        "input_cost_per_token": rates.input_cost_per_token.normalize().to_string(),
        "output_cost_per_token": rates.output_cost_per_token.normalize().to_string(),
        "cache_read_input_token_cost": rates.cache_read_input_token_cost.normalize().to_string(),
        "cache_read_input_audio_token_cost": rates.audio_read.normalize().to_string(),
        "cache_creation_input_token_cost": rates.cache_write.normalize().to_string(),
        "cache_creation_input_token_cost_above_1hr": rates.cache_write_1h.normalize().to_string(),
        "output_cost_per_reasoning_token": rates.reasoning.normalize().to_string(),
    })
}
