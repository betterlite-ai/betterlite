#![cfg(test)]

use super::*;
#[test]
fn cost_parses_sparse_breakdown() {
    let values =
        serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"input_cost":0.2}"#).unwrap();
    let cost = LLMCost::from_breakdown(&values);
    assert_eq!(cost.input, Some(Decimal::new(2, 1)));
    assert_eq!(cost.output, None);
}

#[test]
fn complete_cost_breakdown_retains_every_component() {
    let fields = serde_json::from_str(r#"{"input_cost":1,"output_cost":2,"cache_read_cost":3,"cache_creation_cost":4,"tool_usage_cost":5,"original_cost":6,"discount_amount":7,"discount_percent":8,"margin_fixed_amount":9,"margin_percent":10,"margin_total_amount":11,"additional_costs":{"ignored":12}}"#).unwrap();
    assert_eq!(
        LLMCost::from_breakdown(&fields),
        LLMCost {
            input: Some(Decimal::from(1_i64)),
            output: Some(Decimal::from(2_i64)),
            cache_read: Some(Decimal::from(3_i64)),
            cache_creation: Some(Decimal::from(4_i64)),
            tool_usage: Some(Decimal::from(5_i64)),
            original: Some(Decimal::from(6_i64)),
            discount_amount: Some(Decimal::from(7_i64)),
            discount_percent: Some(Decimal::from(8_i64)),
            margin_fixed_amount: Some(Decimal::from(9_i64)),
            margin_percent: Some(Decimal::from(10_i64)),
            margin_total_amount: Some(Decimal::from(11_i64)),
        }
    );
    assert_eq!(
        LLMCost::from_breakdown(&BTreeMap::new()),
        LLMCost::default()
    );
}
