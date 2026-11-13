#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::controls::control_options::{
    CONTROL_OPTIONS_KEY, ControlOptions, InvalidControlOption, MAX_CONTROL_INT_DIGITS,
    parse_control_options,
};
use crate::controls::json_value::JsonValue;

fn int(value: i64) -> JsonValue {
    JsonValue::Number(Number::from(value))
}

fn uint(value: u64) -> JsonValue {
    JsonValue::Number(Number::from(value))
}

fn float(value: f64) -> JsonValue {
    JsonValue::Number(Number::from_f64(value).expect("a finite test float"))
}

fn text(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

// Nineteen ASCII digits that still parse below the digit limit's value bound, so the length check is the only rejection.
fn padded_nineteen_digits() -> JsonValue {
    JsonValue::Text(format!("{}1", "0".repeat(18)))
}

#[rstest]
#[case::absent(None, Ok(None))]
#[case::explicit_null(Some(JsonValue::Null), Ok(None))]
#[case::positive_integer(Some(uint(512)), Ok(Some(512)))]
#[case::largest_integer(Some(uint(999_999_999_999_999_999)), Ok(Some(999_999_999_999_999_999)))]
#[case::integer_at_the_limit(Some(uint(1_000_000_000_000_000_000)), Err(()))]
#[case::integer_beyond_u64(Some(uint(u64::MAX)), Err(()))]
#[case::zero_integer(Some(int(0)), Err(()))]
#[case::negative_integer(Some(int(-5)), Err(()))]
#[case::whole_float(Some(float(1.0)), Err(()))]
#[case::fractional_float(Some(float(1.5)), Err(()))]
#[case::decimal_string(Some(text("512")), Ok(Some(512)))]
#[case::padded_decimal_string(Some(text("000000000000000001")), Ok(Some(1)))]
#[case::eighteen_digit_string(Some(text("999999999999999999")), Ok(Some(999_999_999_999_999_999)))]
#[case::nineteen_digit_string(Some(text("1111111111111111111")), Err(()))]
#[case::padded_nineteen_digit_string(Some(padded_nineteen_digits()), Err(()))]
#[case::zero_string(Some(text("0")), Err(()))]
#[case::empty_string(Some(text("")), Err(()))]
#[case::leading_space(Some(text(" 5")), Err(()))]
#[case::trailing_space(Some(text("5 ")), Err(()))]
#[case::negative_string(Some(text("-1")), Err(()))]
#[case::plus_sign_string(Some(text("+5")), Err(()))]
#[case::non_ascii_digits(Some(text("\u{661}\u{662}\u{663}")), Err(()))]
#[case::boolean_true(Some(JsonValue::Bool(true)), Err(()))]
#[case::boolean_false(Some(JsonValue::Bool(false)), Err(()))]
#[case::empty_array(Some(JsonValue::Array(Vec::new())), Err(()))]
#[case::empty_object(Some(JsonValue::Object(IndexMap::default())), Err(()))]
fn provider_controls_tests_behavior(
    #[case] given: Option<JsonValue>,
    #[case] expected: Result<Option<u64>, ()>,
) {
    assert_eq!(MAX_CONTROL_INT_DIGITS, 18);
    assert_eq!(CONTROL_OPTIONS_KEY, "_litellm_control");
    if let Ok(size) = expected {
        assert_eq!(
            parse_control_options(given.as_ref()),
            Ok(ControlOptions {
                stream_chunk_size: size
            })
        );
        return;
    }
    let error = parse_control_options(given.as_ref())
        .expect_err("an invalid control option must be rejected");
    assert_eq!(error.param(), "stream_chunk_size");
    let message = error.to_string();
    assert!(message.contains("stream_chunk_size"));
    assert!(message.contains("18 digits"));
}

#[test]
fn constructor_accepts_only_the_valid_range() {
    assert_eq!(
        ControlOptions::new(None),
        Ok(ControlOptions {
            stream_chunk_size: None
        })
    );
    assert_eq!(
        ControlOptions::new(Some(0)),
        Err(InvalidControlOption::StreamChunkSize)
    );
    assert_eq!(
        ControlOptions::new(Some(1_000_000_000_000_000_000)),
        Err(InvalidControlOption::StreamChunkSize)
    );
    assert_eq!(
        ControlOptions::new(Some(999_999_999_999_999_999)),
        Ok(ControlOptions {
            stream_chunk_size: Some(999_999_999_999_999_999)
        })
    );
}
