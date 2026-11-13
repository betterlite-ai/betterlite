#![cfg(test)]
use rstest::rstest;
use rust_decimal::Decimal;

use super::amount_from_text;

#[rstest]
#[case::integer("12", Some(Decimal::new(12, 0)))]
#[case::fraction("1.5", Some(Decimal::new(15, 1)))]
#[case::negative("-2.25", Some(Decimal::new(-225, 2)))]
#[case::small_exponent("1e-5", Some(Decimal::new(1, 5)))]
#[case::capital_exponent("2E3", Some(Decimal::new(2000, 0)))]
#[case::empty("", None)]
#[case::letters("abc", None)]
fn amounts_read_plain_and_scientific_text(#[case] text: &str, #[case] expected: Option<Decimal>) {
    assert_eq!(amount_from_text(text), expected);
}
