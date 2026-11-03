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
