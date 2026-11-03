#![cfg(test)]

use super::*;
#[test]
fn cost_parses_sparse_breakdown() {
    let values =
        serde_json::from_value::<BTreeMap<String, Value>>(serde_json::json!({"input_cost":0.2}))
            .unwrap();
    let cost = LLMCost::from_breakdown(&values);
    assert_eq!(cost.input, Some(0.2));
    assert_eq!(cost.output, None);
}
