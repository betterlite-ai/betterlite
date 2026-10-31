#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{DiscountEntry, MarginEntry};
use rust_decimal::Decimal;

use super::{AdjustmentError, AdjustmentOperation, apply_cost_discount, apply_cost_margin};

fn map<T>(provider: &str, value: T) -> BTreeMap<String, T> {
    BTreeMap::from([(provider.to_owned(), value)])
}

#[test]
fn adjustments_preserve_exact_subcent_values() {
    let discount = apply_cost_discount(
        Decimal::new(1, 2),
        Some("provider"),
        &map("provider", DiscountEntry::Unsupported),
    )
    .unwrap();
    assert_eq!(discount.final_cost, Decimal::new(1, 2));
    let margin = apply_cost_margin(
        Decimal::new(1, 2),
        Some("provider"),
        &map(
            "provider",
            MarginEntry::Split {
                percentage: Some(Decimal::new(1, 1)),
                fixed_amount: Some(Decimal::new(1, 3)),
            },
        ),
    )
    .unwrap();
    assert_eq!(margin.final_cost, Decimal::new(12, 3));
}

#[test]
fn provider_unsupported_margin_shadows_global() {
    let config = BTreeMap::from([
        ("provider".to_owned(), MarginEntry::Unsupported),
        (
            "global".to_owned(),
            MarginEntry::Percent(Decimal::new(5, 1)),
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
        MarginEntry::Percent(Decimal::new(5, 1)),
    )]);
    let result = apply_cost_margin(Decimal::ONE, Some(""), &config).unwrap();
    assert_eq!(result.margin_percent, Decimal::new(5, 1));
    assert_eq!(result.final_cost, Decimal::new(15, 1));
}

#[test]
fn empty_provider_does_not_select_discount() {
    let config = BTreeMap::from([(String::new(), DiscountEntry::Percent(Decimal::new(5, 1)))]);
    let result = apply_cost_discount(Decimal::ONE, Some(""), &config).unwrap();
    assert_eq!(result.discount_percent, Decimal::ZERO);
    assert_eq!(result.saved_amount, Decimal::ZERO);
    assert_eq!(result.final_cost, Decimal::ONE);
}

#[test]
fn unsupported_entries_are_inert() {
    let discount = apply_cost_discount(
        Decimal::ONE,
        Some("provider"),
        &map("provider", DiscountEntry::Unsupported),
    )
    .unwrap();
    assert_eq!(discount.discount_percent, Decimal::ZERO);
    assert_eq!(discount.final_cost, Decimal::ONE);

    let margin = apply_cost_margin(
        Decimal::ONE,
        Some("provider"),
        &map("provider", MarginEntry::Unsupported),
    )
    .unwrap();
    assert_eq!(margin.margin_percent, Decimal::ZERO);
    assert_eq!(margin.final_cost, Decimal::ONE);
}

#[test]
fn fixed_margin_applies_to_zero_base_cost() {
    let config = map(
        "provider",
        MarginEntry::Split {
            percentage: None,
            fixed_amount: Some(Decimal::new(1, 2)),
        },
    );
    let margin = apply_cost_margin(Decimal::ZERO, Some("provider"), &config).unwrap();
    assert_eq!(margin.total_amount, Decimal::new(1, 2));
    assert_eq!(margin.final_cost, Decimal::new(1, 2));
}

#[test]
fn rejects_margin_percentage_overflow() {
    let error = apply_cost_margin(
        Decimal::MAX,
        Some("provider"),
        &map("provider", MarginEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseTimesPercentage
        }
    );
}

#[test]
fn rejects_discount_multiply_overflow() {
    let error = apply_cost_discount(
        Decimal::MAX,
        Some("provider"),
        &map("provider", DiscountEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseTimesDiscount
        }
    );
}

#[test]
fn rejects_discount_subtract_overflow() {
    let error = apply_cost_discount(
        Decimal::MAX,
        Some("provider"),
        &map("provider", DiscountEntry::Percent(Decimal::from(-1_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseMinusSaved
        }
    );
}

#[test]
fn rejects_margin_percentage_plus_fixed_overflow() {
    let config = map(
        "provider",
        MarginEntry::Split {
            percentage: Some(Decimal::from(1_i32)),
            fixed_amount: Some(Decimal::MAX),
        },
    );
    let error = apply_cost_margin(Decimal::MAX, Some("provider"), &config).unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::PercentagePlusFixed
        }
    );
}

#[test]
fn rejects_margin_total_add_overflow() {
    let config = map(
        "provider",
        MarginEntry::Split {
            percentage: None,
            fixed_amount: Some(Decimal::MAX),
        },
    );
    let error = apply_cost_margin(Decimal::MAX, Some("provider"), &config).unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BasePlusMargin
        }
    );
}

#[test]
fn overflow_errors_from_different_operations_are_distinct() {
    let discount_error = apply_cost_discount(
        Decimal::MAX,
        Some("provider"),
        &map("provider", DiscountEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    let margin_error = apply_cost_margin(
        Decimal::MAX,
        Some("provider"),
        &map("provider", MarginEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    assert_ne!(discount_error, margin_error);
}
