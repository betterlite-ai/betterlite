#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{AdjustmentEntry, AdjustmentKind, MarginFieldEntry};
use rust_decimal::Decimal;

use super::{AdjustmentError, AdjustmentOperation, apply_cost_adjustment};

fn map(provider: &str, value: AdjustmentEntry) -> BTreeMap<String, AdjustmentEntry> {
    BTreeMap::from([(provider.to_owned(), value)])
}

#[test]
fn adjustments_preserve_exact_subcent_values() {
    let discount = apply_cost_adjustment(
        AdjustmentKind::Discount,
        Decimal::new(1, 2),
        Some("provider"),
        &map("provider", AdjustmentEntry::Unsupported),
    )
    .unwrap();
    assert_eq!(discount.final_cost, Decimal::new(1, 2));
    let margin = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::new(1, 2),
        Some("provider"),
        &map(
            "provider",
            AdjustmentEntry::Split {
                percentage: MarginFieldEntry::Number(Decimal::new(1, 1)),
                fixed_amount: MarginFieldEntry::Number(Decimal::new(1, 3)),
            },
        ),
    )
    .unwrap();
    assert_eq!(margin.final_cost, Decimal::new(12, 3));
}

#[test]
fn provider_unsupported_margin_shadows_global() {
    let config = BTreeMap::from([
        ("provider".to_owned(), AdjustmentEntry::Unsupported),
        (
            "global".to_owned(),
            AdjustmentEntry::Percent(Decimal::new(5, 1)),
        ),
    ]);
    assert_eq!(
        apply_cost_adjustment(
            AdjustmentKind::Margin,
            Decimal::ONE,
            Some("provider"),
            &config
        )
        .unwrap()
        .final_cost,
        Decimal::ONE
    );
}

#[test]
fn empty_provider_uses_global_margin() {
    let config = BTreeMap::from([(
        "global".to_owned(),
        AdjustmentEntry::Percent(Decimal::new(5, 1)),
    )]);
    let result =
        apply_cost_adjustment(AdjustmentKind::Margin, Decimal::ONE, Some(""), &config).unwrap();
    assert_eq!(result.percent, Decimal::new(5, 1));
    assert_eq!(result.final_cost, Decimal::new(15, 1));
}

#[test]
fn empty_provider_does_not_select_discount() {
    let config = BTreeMap::from([(String::new(), AdjustmentEntry::Percent(Decimal::new(5, 1)))]);
    let result =
        apply_cost_adjustment(AdjustmentKind::Discount, Decimal::ONE, Some(""), &config).unwrap();
    assert_eq!(result.percent, Decimal::ZERO);
    assert_eq!(result.amount, Decimal::ZERO);
    assert_eq!(result.final_cost, Decimal::ONE);
}

#[test]
fn unsupported_entries_are_inert() {
    let discount = apply_cost_adjustment(
        AdjustmentKind::Discount,
        Decimal::ONE,
        Some("provider"),
        &map("provider", AdjustmentEntry::Unsupported),
    )
    .unwrap();
    assert_eq!(discount.percent, Decimal::ZERO);
    assert_eq!(discount.final_cost, Decimal::ONE);

    let margin = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::ONE,
        Some("provider"),
        &map("provider", AdjustmentEntry::Unsupported),
    )
    .unwrap();
    assert_eq!(margin.percent, Decimal::ZERO);
    assert_eq!(margin.final_cost, Decimal::ONE);
}

#[test]
fn fixed_margin_applies_to_zero_base_cost() {
    let config = map(
        "provider",
        AdjustmentEntry::Split {
            percentage: MarginFieldEntry::Absent,
            fixed_amount: MarginFieldEntry::Number(Decimal::new(1, 2)),
        },
    );
    let margin = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::ZERO,
        Some("provider"),
        &config,
    )
    .unwrap();
    assert_eq!(margin.amount, Decimal::new(1, 2));
    assert_eq!(margin.final_cost, Decimal::new(1, 2));
}

#[test]
fn rejects_margin_percentage_overflow() {
    let error = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::MAX,
        Some("provider"),
        &map("provider", AdjustmentEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseTimesPercent
        }
    );
}

#[test]
fn rejects_discount_multiply_overflow() {
    let error = apply_cost_adjustment(
        AdjustmentKind::Discount,
        Decimal::MAX,
        Some("provider"),
        &map("provider", AdjustmentEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseTimesPercent
        }
    );
}

#[test]
fn rejects_discount_subtract_overflow() {
    let error = apply_cost_adjustment(
        AdjustmentKind::Discount,
        Decimal::MAX,
        Some("provider"),
        &map("provider", AdjustmentEntry::Percent(Decimal::from(-1_i32))),
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BaseMinusAmount
        }
    );
}

#[test]
fn rejects_margin_percentage_plus_fixed_overflow() {
    let config = map(
        "provider",
        AdjustmentEntry::Split {
            percentage: MarginFieldEntry::Number(Decimal::from(1_i32)),
            fixed_amount: MarginFieldEntry::Number(Decimal::MAX),
        },
    );
    let error = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::MAX,
        Some("provider"),
        &config,
    )
    .unwrap_err();
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
        AdjustmentEntry::Split {
            percentage: MarginFieldEntry::Absent,
            fixed_amount: MarginFieldEntry::Number(Decimal::MAX),
        },
    );
    let error = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::MAX,
        Some("provider"),
        &config,
    )
    .unwrap_err();
    assert_eq!(
        error,
        AdjustmentError::Overflow {
            operation: AdjustmentOperation::BasePlusAmount
        }
    );
}

#[test]
fn overflow_errors_from_different_operations_are_distinct() {
    let discount_error = apply_cost_adjustment(
        AdjustmentKind::Discount,
        Decimal::MAX,
        Some("provider"),
        &map("provider", AdjustmentEntry::Percent(Decimal::from(-1_i32))),
    )
    .unwrap_err();
    let margin_error = apply_cost_adjustment(
        AdjustmentKind::Margin,
        Decimal::MAX,
        Some("provider"),
        &map("provider", AdjustmentEntry::Percent(Decimal::from(2_i32))),
    )
    .unwrap_err();
    assert_ne!(discount_error, margin_error);
}
