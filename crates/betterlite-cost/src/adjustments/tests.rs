#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde_json::json;

use super::{AdjustmentError, CostAdjustmentMap, apply_cost_discount, apply_cost_margin};

fn map(value: serde_json::Value) -> CostAdjustmentMap {
    BTreeMap::from([("provider".to_owned(), value)])
}

#[test]
fn adjustments_preserve_exact_subcent_values() {
    let discount =
        apply_cost_discount(Decimal::new(1, 2), Some("provider"), &map(json!("0.2"))).unwrap();
    assert_eq!(discount.final_cost, Decimal::new(1, 2));
    let margin = apply_cost_margin(
        Decimal::new(1, 2),
        Some("provider"),
        &map(json!({"percentage": "0.1", "fixed_amount": "0.001"})),
    )
    .unwrap();
    assert_eq!(margin.final_cost, Decimal::new(12, 3));
}

#[test]
fn provider_null_margin_shadows_global() {
    let config = BTreeMap::from([
        ("provider".to_owned(), serde_json::Value::Null),
        ("global".to_owned(), json!(0.5)),
    ]);
    assert_eq!(
        apply_cost_margin(Decimal::ONE, Some("provider"), &config)
            .unwrap()
            .final_cost,
        Decimal::ONE
    );
}

#[test]
fn rejects_overflow() {
    let error = apply_cost_margin(Decimal::MAX, Some("provider"), &map(json!(2))).unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: "base * percentage"
        }
    );
}
