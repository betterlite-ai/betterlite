#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::CostAdjustmentValue;
use rust_decimal::Decimal;

use super::{AdjustmentError, CostAdjustmentMap, apply_cost_discount, apply_cost_margin};

fn map(value: CostAdjustmentValue) -> CostAdjustmentMap {
    BTreeMap::from([("provider".to_owned(), value)])
}

#[test]
fn adjustments_preserve_exact_subcent_values() {
    let discount = apply_cost_discount(
        Decimal::new(1, 2),
        Some("provider"),
        &map(CostAdjustmentValue::Text("0.2".to_owned())),
    )
    .unwrap();
    assert_eq!(discount.final_cost, Decimal::new(1, 2));
    let margin = apply_cost_margin(
        Decimal::new(1, 2),
        Some("provider"),
        &map(CostAdjustmentValue::Object(BTreeMap::from([
            (
                "percentage".to_owned(),
                CostAdjustmentValue::Text("0.1".to_owned()),
            ),
            (
                "fixed_amount".to_owned(),
                CostAdjustmentValue::Text("0.001".to_owned()),
            ),
        ]))),
    )
    .unwrap();
    assert_eq!(margin.final_cost, Decimal::new(12, 3));
}

#[test]
fn provider_null_margin_shadows_global() {
    let config = BTreeMap::from([
        ("provider".to_owned(), CostAdjustmentValue::Null),
        (
            "global".to_owned(),
            CostAdjustmentValue::Number(serde_json::Number::from_f64(0.5).unwrap()),
        ),
    ]);
    assert_eq!(
        apply_cost_margin(Decimal::ONE, Some("provider"), &config)
            .unwrap()
            .final_cost,
        Decimal::ONE
    );
}

#[test]
fn empty_provider_uses_global_margin() {
    let config = BTreeMap::from([(
        "global".to_owned(),
        CostAdjustmentValue::Number(serde_json::Number::from_f64(0.5).unwrap()),
    )]);
    let result = apply_cost_margin(Decimal::ONE, Some(""), &config).unwrap();
    assert_eq!(result.margin_percent, Decimal::new(5, 1));
    assert_eq!(result.final_cost, Decimal::new(15, 1));
}

#[test]
fn empty_provider_does_not_select_discount() {
    let config = BTreeMap::from([(
        String::new(),
        CostAdjustmentValue::Number(serde_json::Number::from_f64(0.5).unwrap()),
    )]);
    let result = apply_cost_discount(Decimal::ONE, Some(""), &config).unwrap();
    assert_eq!(result.discount_percent, Decimal::ZERO);
    assert_eq!(result.saved_amount, Decimal::ZERO);
    assert_eq!(result.final_cost, Decimal::ONE);
}

#[test]
fn boolean_config_values_are_inert() {
    let discount = apply_cost_discount(
        Decimal::ONE,
        Some("provider"),
        &map(CostAdjustmentValue::Boolean(true)),
    )
    .unwrap();
    assert_eq!(discount.discount_percent, Decimal::ZERO);
    assert_eq!(discount.final_cost, Decimal::ONE);

    let margin = apply_cost_margin(
        Decimal::ONE,
        Some("provider"),
        &map(CostAdjustmentValue::Boolean(true)),
    )
    .unwrap();
    assert_eq!(margin.margin_percent, Decimal::ZERO);
    assert_eq!(margin.final_cost, Decimal::ONE);
}

#[test]
fn fixed_margin_applies_to_zero_base_cost() {
    let config = map(CostAdjustmentValue::Object(BTreeMap::from([(
        "fixed_amount".to_owned(),
        CostAdjustmentValue::Number(serde_json::Number::from_f64(0.01).unwrap()),
    )])));
    let margin = apply_cost_margin(Decimal::ZERO, Some("provider"), &config).unwrap();
    assert_eq!(margin.total_amount, Decimal::new(1, 2));
    assert_eq!(margin.final_cost, Decimal::new(1, 2));
}

#[test]
fn rejects_margin_percentage_overflow() {
    let error = apply_cost_margin(
        Decimal::MAX,
        Some("provider"),
        &map(CostAdjustmentValue::Number(serde_json::Number::from(2_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: "base * percentage"
        }
    );
}

#[test]
fn rejects_discount_multiply_overflow() {
    let error = apply_cost_discount(
        Decimal::MAX,
        Some("provider"),
        &map(CostAdjustmentValue::Number(serde_json::Number::from(2_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: "base * discount"
        }
    );
}

#[test]
fn rejects_discount_subtract_overflow() {
    let error = apply_cost_discount(
        Decimal::MAX,
        Some("provider"),
        &map(CostAdjustmentValue::Number(serde_json::Number::from(
            -1_i32,
        ))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: "base - saved"
        }
    );
}

#[test]
fn rejects_margin_percentage_plus_fixed_overflow() {
    let config = map(CostAdjustmentValue::Object(BTreeMap::from([
        (
            "percentage".to_owned(),
            CostAdjustmentValue::Number(serde_json::Number::from(1_i32)),
        ),
        (
            "fixed_amount".to_owned(),
            CostAdjustmentValue::Text(Decimal::MAX.to_string()),
        ),
    ])));
    let error = apply_cost_margin(Decimal::MAX, Some("provider"), &config).unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: "percentage + fixed"
        }
    );
}

#[test]
fn rejects_margin_total_add_overflow() {
    let config = map(CostAdjustmentValue::Object(BTreeMap::from([(
        "fixed_amount".to_owned(),
        CostAdjustmentValue::Text(Decimal::MAX.to_string()),
    )])));
    let error = apply_cost_margin(Decimal::MAX, Some("provider"), &config).unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: "base + margin"
        }
    );
}
