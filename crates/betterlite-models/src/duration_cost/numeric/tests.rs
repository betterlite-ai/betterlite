#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::numeric_price;
use crate::price_lookup::PriceRecord;
use crate::tiered_pricing::Scalar;

fn record_with(key: &str, value: Scalar) -> PriceRecord {
    let mut record = PriceRecord::new();
    record.insert(key.to_owned(), value);
    record
}

fn number(text: &str) -> Scalar {
    Scalar::Number(
        Number::from_f64(text.parse().expect("test literal is numeric")).expect("finite"),
    )
}

#[rstest]
#[case::absent(None, None)]
#[case::null(Some(Scalar::Null), None)]
#[case::zero(Some(number("0")), Some(Decimal::ZERO))]
#[case::positive(Some(number("1.5")), Some(Decimal::new(15, 1)))]
#[case::boolean(Some(Scalar::Bool(true)), Some(Decimal::ONE))]
fn numeric_price_distinguishes_absence_from_a_present_value(
    #[case] stored: Option<Scalar>,
    #[case] expected: Option<Decimal>,
) {
    let record = stored.map_or_else(PriceRecord::new, |value| {
        record_with("output_cost_per_second", value)
    });
    assert_eq!(
        numeric_price(&record, "output_cost_per_second").expect("coercible"),
        expected
    );
}

#[test]
fn numeric_price_rejects_an_array_as_a_typed_error() {
    let record = record_with("output_cost_per_second", Scalar::Array(Vec::new()));
    numeric_price(&record, "output_cost_per_second").unwrap_err();
}
