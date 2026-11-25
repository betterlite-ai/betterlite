use rstest::rstest;
use rust_decimal::Decimal;

use crate::auth_key_budgets::money::Money;

use super::{dec, dec_text, json, money};

#[test]
fn money_follows_float_semantics() {
    assert_eq!(Money::from_f64(f64::NAN), Money::Nan);
    assert_eq!(Money::from_f64(f64::INFINITY), Money::PositiveInfinity);
    assert_eq!(Money::from_f64(f64::NEG_INFINITY), Money::NegativeInfinity);
    assert_eq!(Money::from_f64(0.5_f64), Money::Finite(dec_text("0.5")));
    assert_eq!(Money::from_f64(1e300_f64), Money::PositiveInfinity);
    assert_eq!(Money::from_f64(-1e300_f64), Money::NegativeInfinity);
}

#[test]
fn money_text_and_json_semantics() {
    assert_eq!(Money::from_text("NaN"), Some(Money::Nan));
    assert_eq!(Money::from_text("Infinity"), Some(Money::PositiveInfinity));
    assert_eq!(Money::from_text("-Infinity"), Some(Money::NegativeInfinity));
    assert_eq!(Money::from_text("5"), Some(money(5)));
    assert_eq!(Money::from_text("1e999"), Some(Money::PositiveInfinity));
    assert_eq!(Money::from_text("-1e999"), Some(Money::NegativeInfinity));
    assert_eq!(Money::from_text("abc"), None);
    assert_eq!(Money::from_json(&json("5")), Some(money(5)));
    assert_eq!(Money::from_json(&json("\"5\"")), None);
    assert_eq!(Money::from_json(&json("true")), None);
    assert_eq!(
        Money::from_json(&json("\"Infinity\"")),
        Some(Money::PositiveInfinity)
    );
}

#[test]
fn spelled_nonfinite_forms_are_exact() {
    assert_eq!(Money::spelled("NaN"), Some(Money::Nan));
    assert_eq!(Money::spelled("Infinity"), Some(Money::PositiveInfinity));
    assert_eq!(Money::spelled("-Infinity"), Some(Money::NegativeInfinity));
    assert_eq!(Money::spelled("inf"), None);
    assert_eq!(Money::spelled("nan"), None);
    assert_eq!(Money::spelled("-7"), None);
}

#[test]
fn money_percent_and_finite_semantics() {
    assert_eq!(money(100).mul_percent(50), money(50));
    assert_eq!(money(100).mul_decimal_percent(50.0_f64), money(50));
    assert_eq!(
        money(100).mul_decimal_percent(f64::INFINITY),
        Money::PositiveInfinity
    );
    assert_eq!(money(100).mul_decimal_percent(f64::NAN), Money::Nan);
    assert_eq!(money(5).finite_value(), Some(dec(5)));
    assert_eq!(Money::Nan.finite_value(), None);
}

#[test]
fn money_comparison_semantics() {
    assert!(Money::spend_at_least(dec(5), money(5)));
    assert!(!Money::spend_at_least(dec(5), money(6)));
    assert!(!Money::spend_at_least(dec(5), Money::Nan));
    assert!(!Money::spend_at_least(dec(5), Money::PositiveInfinity));
    assert!(Money::spend_at_least(dec(5), Money::NegativeInfinity));
    assert!(!Money::spend_below(dec(5), money(5)));
    assert!(Money::spend_below(dec(5), money(6)));
    assert!(!Money::spend_below(dec(5), Money::Nan));
    assert!(Money::spend_below(dec(5), Money::PositiveInfinity));
    assert!(!Money::spend_below(dec(5), Money::NegativeInfinity));
}

#[rstest]
#[case::finite(money(2), money(3), money(5))]
#[case::left_nan(Money::Nan, money(1), Money::Nan)]
#[case::right_nan(money(1), Money::Nan, Money::Nan)]
#[case::left_positive(Money::PositiveInfinity, money(1), Money::PositiveInfinity)]
#[case::right_positive(money(1), Money::PositiveInfinity, Money::PositiveInfinity)]
#[case::left_negative(Money::NegativeInfinity, money(1), Money::NegativeInfinity)]
#[case::right_negative(money(1), Money::NegativeInfinity, Money::NegativeInfinity)]
#[case::opposite(Money::PositiveInfinity, Money::NegativeInfinity, Money::Nan)]
#[case::reversed_opposite(Money::NegativeInfinity, Money::PositiveInfinity, Money::Nan)]
#[case::both_positive(
    Money::PositiveInfinity,
    Money::PositiveInfinity,
    Money::PositiveInfinity
)]
#[case::both_negative(
    Money::NegativeInfinity,
    Money::NegativeInfinity,
    Money::NegativeInfinity
)]
#[case::max_overflow(
    Money::Finite(Decimal::MAX),
    Money::Finite(Decimal::MAX),
    Money::PositiveInfinity
)]
#[case::min_overflow(
    Money::Finite(Decimal::MIN),
    Money::Finite(Decimal::MIN),
    Money::NegativeInfinity
)]
fn money_add_follows_float_semantics(
    #[case] left: Money,
    #[case] right: Money,
    #[case] sum: Money,
) {
    assert_eq!(left.plus(right), sum);
}

#[rstest]
#[case::finite(money(2), money(3), money(6))]
#[case::left_nan(Money::Nan, money(1), Money::Nan)]
#[case::right_nan(money(1), Money::Nan, Money::Nan)]
#[case::both_positive(
    Money::PositiveInfinity,
    Money::PositiveInfinity,
    Money::PositiveInfinity
)]
#[case::both_negative(
    Money::NegativeInfinity,
    Money::NegativeInfinity,
    Money::PositiveInfinity
)]
#[case::positive_negative(
    Money::PositiveInfinity,
    Money::NegativeInfinity,
    Money::NegativeInfinity
)]
#[case::negative_positive(
    Money::NegativeInfinity,
    Money::PositiveInfinity,
    Money::NegativeInfinity
)]
#[case::positive_times_two(Money::PositiveInfinity, money(2), Money::PositiveInfinity)]
#[case::positive_times_negative_two(
    Money::PositiveInfinity,
    money(-2),
    Money::NegativeInfinity
)]
#[case::two_times_positive(money(2), Money::PositiveInfinity, Money::PositiveInfinity)]
#[case::negative_two_times_positive(
    money(-2),
    Money::PositiveInfinity,
    Money::NegativeInfinity
)]
#[case::negative_times_two(Money::NegativeInfinity, money(2), Money::NegativeInfinity)]
#[case::negative_times_negative_two(
    Money::NegativeInfinity,
    money(-2),
    Money::PositiveInfinity
)]
#[case::two_times_negative(money(2), Money::NegativeInfinity, Money::NegativeInfinity)]
#[case::negative_two_times_negative(
    money(-2),
    Money::NegativeInfinity,
    Money::PositiveInfinity
)]
#[case::positive_times_zero(Money::PositiveInfinity, money(0), Money::Nan)]
#[case::negative_times_zero(Money::NegativeInfinity, money(0), Money::Nan)]
#[case::max_overflow(Money::Finite(Decimal::MAX), money(2), Money::PositiveInfinity)]
#[case::min_overflow(Money::Finite(Decimal::MIN), money(2), Money::NegativeInfinity)]
fn money_mul_follows_float_semantics(
    #[case] left: Money,
    #[case] right: Money,
    #[case] product: Money,
) {
    assert_eq!(left.times(right), product);
}
