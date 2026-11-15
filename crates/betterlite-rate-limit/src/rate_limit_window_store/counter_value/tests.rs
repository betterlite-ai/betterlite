#![cfg(test)]

use rstest::rstest;

use super::{CounterValueError, counter_as_int};
use crate::limiter_records::CacheCounterValue;

#[rstest]
#[case::integer(CacheCounterValue::Int(7), 7)]
#[case::text(CacheCounterValue::Text("7".to_owned()), 7)]
#[case::padded_text(CacheCounterValue::Text(" 7 ".to_owned()), 7)]
#[case::bytes(CacheCounterValue::Bytes(b"7".to_vec()), 7)]
#[case::negative_text(CacheCounterValue::Text("-7".to_owned()), -7)]
fn a_whole_number_counter_reads_as_itself(#[case] value: CacheCounterValue, #[case] expected: i64) {
    assert_eq!(counter_as_int(&value).expect("a whole number"), expected);
}

#[rstest]
#[case::positive_tail(CacheCounterValue::Float(12.9), 12)]
#[case::negative_tail(CacheCounterValue::Float(-12.9), -12)]
#[case::whole_float(CacheCounterValue::Float(12.0), 12)]
fn a_float_counter_is_cut_toward_zero(#[case] value: CacheCounterValue, #[case] expected: i64) {
    assert_eq!(counter_as_int(&value).expect("a whole number"), expected);
}

#[rstest]
#[case::text(CacheCounterValue::Text("soon".to_owned()), "text")]
#[case::bytes(CacheCounterValue::Bytes(b"soon".to_vec()), "bytes")]
#[case::fraction_beyond_range(CacheCounterValue::Float(1e30), "float")]
#[case::not_a_number(CacheCounterValue::Float(f64::NAN), "float")]
fn a_counter_that_is_not_a_whole_number_is_an_error(
    #[case] value: CacheCounterValue,
    #[case] named: &str,
) {
    let error = counter_as_int(&value).expect_err("a counter with no whole number");
    let named_error = match &error {
        CounterValueError::Text(_) => "text",
        CounterValueError::Bytes(_) => "bytes",
        CounterValueError::Utf8(_) => "utf8",
        CounterValueError::Float { .. } => "float",
    };
    assert_eq!(named_error, named);
}

#[test]
fn counter_bytes_that_are_not_utf8_name_the_encoding_error() {
    let error = counter_as_int(&CacheCounterValue::Bytes(vec![0xFF]))
        .expect_err("bytes that are not UTF-8");
    assert!(matches!(error, CounterValueError::Utf8(_)));
}
