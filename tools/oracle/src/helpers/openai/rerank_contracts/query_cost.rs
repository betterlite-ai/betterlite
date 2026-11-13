use betterlite_provider::api::rerank_contracts::BaseRerankConfig as _;
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};

use super::recording::Recording;
use super::{ContractError, CostParams};

pub(super) fn run(params: &CostParams) -> Result<Value, ContractError> {
    let config = Recording::default();
    let mut costs = Vec::new();
    for case in &params.cases {
        let (prompt, completion) = config.calculate_rerank_cost(
            &params.model,
            None,
            case.billed_units.as_ref(),
            case.model_info.as_ref(),
        )?;
        costs.push(json!([decimal_number(prompt), decimal_number(completion)]));
    }
    Ok(json!({"costs": costs}))
}

fn decimal_number(decimal: Decimal) -> Value {
    let mut text = decimal.normalize().to_string();
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    text.parse::<Number>().map_or(Value::Null, Value::Number)
}
