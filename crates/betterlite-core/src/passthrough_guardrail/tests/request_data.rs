use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::data;
use crate::passthrough_guardrail::errors::PassthroughPayloadError;
use crate::passthrough_guardrail::wire::parse_request_data;

#[test]
fn parse_request_data_keeps_non_integer_numbers_as_decimals() {
    let parsed = data(r#"{"temperature": 0.7, "nested": {"weights": [1.5, 2]}}"#);
    let nested = GuardrailValue::Object(BTreeMap::from([(
        "weights".to_owned(),
        GuardrailValue::Array(vec![
            GuardrailValue::Decimal(1.5),
            GuardrailValue::SignedInteger(2),
        ]),
    )]));
    assert_eq!(
        parsed,
        BTreeMap::from([
            ("temperature".to_owned(), GuardrailValue::Decimal(0.7)),
            ("nested".to_owned(), nested),
        ])
    );
}

#[test]
fn parse_request_data_keeps_integers_of_every_sign_and_width() {
    let parsed = data(r#"{"small": -3, "wide": 18446744073709551615, "huge": 1e300}"#);
    assert_eq!(
        parsed,
        BTreeMap::from([
            ("small".to_owned(), GuardrailValue::SignedInteger(-3)),
            ("wide".to_owned(), GuardrailValue::UnsignedInteger(u64::MAX)),
            ("huge".to_owned(), GuardrailValue::Decimal(1e300)),
        ])
    );
}

#[test]
fn parse_request_data_reads_a_lone_private_key_as_the_number_it_carries() {
    let parsed = data(
        r#"{"a": {"$serde_json::private::Number": "-5"}, "b": {"$serde_json::private::Number": "18446744073709551615"}}"#,
    );
    assert_eq!(
        parsed,
        BTreeMap::from([
            ("a".to_owned(), GuardrailValue::SignedInteger(-5)),
            ("b".to_owned(), GuardrailValue::UnsignedInteger(u64::MAX)),
        ])
    );
}

#[test]
fn parse_request_data_keeps_a_private_key_object_that_is_no_number() {
    let parsed = data(
        r#"{"a": {"$serde_json::private::Number": "0.5", "other": 1}, "b": {"$serde_json::private::Number": "abc"}}"#,
    );
    let key = "$serde_json::private::Number".to_owned();
    let with_sibling = GuardrailValue::Object(BTreeMap::from([
        (key.clone(), GuardrailValue::Text("0.5".to_owned())),
        ("other".to_owned(), GuardrailValue::SignedInteger(1)),
    ]));
    let not_a_number = GuardrailValue::Object(BTreeMap::from([(
        key,
        GuardrailValue::Text("abc".to_owned()),
    )]));
    assert_eq!(
        parsed,
        BTreeMap::from([
            ("a".to_owned(), with_sibling),
            ("b".to_owned(), not_a_number)
        ])
    );
}

#[rstest]
#[case::positive(
    r#"{"id": 123456789012345678901234567890}"#,
    "123456789012345678901234567890"
)]
#[case::negative(
    r#"{"id": -123456789012345678901234567890}"#,
    "-123456789012345678901234567890"
)]
#[case::just_above_u64(r#"{"id": 18446744073709551616}"#, "18446744073709551616")]
#[case::just_below_i64(r#"{"id": -9223372036854775809}"#, "-9223372036854775809")]
#[case::nested(
    r#"{"ids": [{"id": 123456789012345678901234567890}]}"#,
    "123456789012345678901234567890"
)]
fn parse_request_data_refuses_an_integer_beyond_the_64_bit_range(
    #[case] source: &str,
    #[case] digits: &str,
) {
    let Err(PassthroughPayloadError::IntegerOutOfRange { text }) = parse_request_data(source)
    else {
        panic!("an integer wider than 64 bits must be refused");
    };
    assert_eq!(text, digits);
}

#[test]
fn parse_request_data_keeps_a_wide_number_with_a_fraction_or_exponent_as_a_decimal() {
    let parsed = data(r#"{"fraction": 123456789012345678901234567890.5, "exponent": 1.5e30}"#);
    assert_eq!(
        parsed,
        BTreeMap::from([
            (
                "fraction".to_owned(),
                GuardrailValue::Decimal(1.234_567_890_123_456_8e29)
            ),
            ("exponent".to_owned(), GuardrailValue::Decimal(1.5e30)),
        ])
    );
}

#[test]
fn parse_request_data_rejects_a_payload_that_is_not_an_object() {
    assert!(matches!(
        parse_request_data("[1, 2]"),
        Err(PassthroughPayloadError::Parse(_))
    ));
}
