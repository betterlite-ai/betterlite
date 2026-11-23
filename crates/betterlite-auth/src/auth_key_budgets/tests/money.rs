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

    for (left, right, sum) in [
        (money(2), money(3), money(5)),
        (Money::Nan, money(1), Money::Nan),
        (money(1), Money::Nan, Money::Nan),
        (Money::PositiveInfinity, money(1), Money::PositiveInfinity),
        (money(1), Money::PositiveInfinity, Money::PositiveInfinity),
        (Money::NegativeInfinity, money(1), Money::NegativeInfinity),
        (money(1), Money::NegativeInfinity, Money::NegativeInfinity),
        (Money::PositiveInfinity, Money::NegativeInfinity, Money::Nan),
        (Money::NegativeInfinity, Money::PositiveInfinity, Money::Nan),
        (
            Money::PositiveInfinity,
            Money::PositiveInfinity,
            Money::PositiveInfinity,
        ),
        (
            Money::NegativeInfinity,
            Money::NegativeInfinity,
            Money::NegativeInfinity,
        ),
        (
            Money::Finite(Decimal::MAX),
            Money::Finite(Decimal::MAX),
            Money::PositiveInfinity,
        ),
        (
            Money::Finite(Decimal::MIN),
            Money::Finite(Decimal::MIN),
            Money::NegativeInfinity,
        ),
    ] {
        assert_eq!(left.add(right), sum);
    }
    for (left, right, product) in [
        (money(2), money(3), money(6)),
        (Money::Nan, money(1), Money::Nan),
        (money(1), Money::Nan, Money::Nan),
        (
            Money::PositiveInfinity,
            Money::PositiveInfinity,
            Money::PositiveInfinity,
        ),
        (
            Money::NegativeInfinity,
            Money::NegativeInfinity,
            Money::PositiveInfinity,
        ),
        (
            Money::PositiveInfinity,
            Money::NegativeInfinity,
            Money::NegativeInfinity,
        ),
        (
            Money::NegativeInfinity,
            Money::PositiveInfinity,
            Money::NegativeInfinity,
        ),
        (Money::PositiveInfinity, money(2), Money::PositiveInfinity),
        (Money::PositiveInfinity, money(-2), Money::NegativeInfinity),
        (money(2), Money::PositiveInfinity, Money::PositiveInfinity),
        (money(-2), Money::PositiveInfinity, Money::NegativeInfinity),
        (Money::NegativeInfinity, money(2), Money::NegativeInfinity),
        (Money::NegativeInfinity, money(-2), Money::PositiveInfinity),
        (money(2), Money::NegativeInfinity, Money::NegativeInfinity),
        (money(-2), Money::NegativeInfinity, Money::PositiveInfinity),
        (Money::PositiveInfinity, money(0), Money::Nan),
        (Money::NegativeInfinity, money(0), Money::Nan),
        (
            Money::Finite(Decimal::MAX),
            money(2),
            Money::PositiveInfinity,
        ),
        (
            Money::Finite(Decimal::MIN),
            money(2),
            Money::NegativeInfinity,
        ),
    ] {
        assert_eq!(left.mul(right), product);
    }
    assert_eq!(money(100).mul_percent(50), money(50));
    assert_eq!(money(100).mul_decimal_percent(50.0_f64), money(50));
    assert_eq!(
        money(100).mul_decimal_percent(f64::INFINITY),
        Money::PositiveInfinity
    );
    assert_eq!(money(100).mul_decimal_percent(f64::NAN), Money::Nan);
    assert!(!money(0).truthy());
    assert!(money(1).truthy());
    assert!(Money::Nan.truthy());
    assert!(Money::PositiveInfinity.truthy());
    assert!(Money::NegativeInfinity.truthy());
    assert_eq!(money(5).finite_value(), Some(dec(5)));
    assert_eq!(Money::Nan.finite_value(), None);
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
