#![cfg(test)]

use super::{FloatInput, IntInput, ValueLookup};
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::HashMap;

#[rstest]
#[case(2_i32, Decimal::from(2_i32))]
#[case(2.5_f64, Decimal::new(25, 1))]
#[case(" 2.5\n", Decimal::new(25, 1))]
#[case(b"2.5".as_slice(), Decimal::new(25, 1))]
#[case(b"1_000.25".to_vec(), Decimal::new(100_025, 2))]
#[case("1e-05", Decimal::new(1, 5))]
#[case(true, Decimal::ONE)]
#[case(false, Decimal::ZERO)]
fn to_float_accepts_values<T>(#[case] value: T, #[case] expected: Decimal)
where
    T: FloatInput,
{
    assert_eq!(value.decimal_value(), Some(expected));
}

#[rstest]
#[case("_5")]
#[case("5_")]
#[case("1__0")]
fn to_float_rejects_invalid_digit_separators(#[case] value: &str) {
    assert_eq!(value.decimal_value(), None);
}

#[test]
fn to_float_rejects_u128_values_decimal_cannot_hold() {
    assert_eq!(u128::MAX.decimal_value(), None);
}

#[test]
fn to_float_rejects_non_finite_values() {
    assert_eq!(f64::INFINITY.decimal_value(), None);
    assert_eq!(f64::NAN.decimal_value(), None);
}

#[test]
fn to_float_rejects_non_utf8_bytes() {
    assert_eq!(vec![0xff].decimal_value(), None);
    assert_eq!([0xff].as_slice().decimal_value(), None);
}

#[rstest]
#[case(2_i32, 2_i64)]
#[case(2.9_f64, 2_i64)]
#[case(" 1_000\n", 1_000_i64)]
#[case(b" 1_000\n".to_vec(), 1_000_i64)]
#[case(true, 1_i64)]
#[case(false, 0_i64)]
fn to_int_accepts_values<T>(#[case] value: T, #[case] expected: i64)
where
    T: IntInput,
{
    assert_eq!(value.int_value(), Some(expected));
}

#[rstest]
#[case("_5")]
#[case("5_")]
#[case("1__0")]
#[case("2.9")]
fn to_int_rejects_invalid_digit_separators(#[case] value: &str) {
    assert_eq!(value.int_value(), None);
}

#[test]
fn to_int_rejects_non_finite_and_out_of_range_values() {
    assert_eq!(f64::NAN.int_value(), None);
    assert_eq!(f64::INFINITY.int_value(), None);
    assert_eq!(u64::MAX.int_value(), None);
    assert_eq!("9223372036854775808".int_value(), None);
}

#[test]
fn to_int_rejects_non_utf8_bytes() {
    assert_eq!(vec![0xff].int_value(), None);
    assert_eq!([0xff].as_slice().int_value(), None);
}

struct Object(String);

impl ValueLookup for Object {
    fn value(&self, key: &str) -> Option<&String> {
        (key == "spend").then_some(&self.0)
    }
}

#[test]
fn get_value_reads_mapping_and_object_without_conversion() {
    let values = HashMap::from([(String::from("spend"), String::from("2.5"))]);
    assert_eq!(values.value("spend"), Some(&String::from("2.5")));
    assert_eq!(values.value("missing"), None);
    assert_eq!(
        Object(String::from("3")).value("spend"),
        Some(&String::from("3"))
    );
}
