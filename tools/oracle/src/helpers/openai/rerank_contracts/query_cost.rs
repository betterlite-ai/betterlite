use betterlite_provider::api::rerank_contracts::BaseRerankConfig as _;
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};

use super::CostParams;
use super::recording::Recording;

pub(super) fn run(params: &CostParams) -> Value {
    let config = Recording::default();
    let mut costs = Vec::new();
    for case in &params.cases {
        let (prompt, completion) = config.calculate_rerank_cost(
            &params.model,
            None,
            case.billed_units.as_ref(),
            case.model_info.as_ref(),
        );
        costs.push(json!([decimal_number(prompt), decimal_number(completion)]));
    }
    json!({"costs": costs})
}

fn decimal_number(decimal: Decimal) -> Value {
    let mut text = decimal.normalize().to_string();
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    text.parse::<Number>().map_or(Value::Null, Value::Number)
}
