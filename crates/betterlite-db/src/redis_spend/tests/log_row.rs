use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::redis_spend::log_row::{SpendLogError, SpendLogValue, decode_spend_log_row};

fn dec(text: &str) -> Decimal {
    text.parse::<Decimal>().unwrap()
}

fn moment(text: &str) -> DateTime<Utc> {
    text.parse::<DateTime<Utc>>().unwrap()
}

fn encoded(value: &SpendLogValue) -> String {
    serde_json::to_string(value).unwrap()
}

#[test]
fn a_datetime_column_becomes_its_isoformat_text() {
    let with_micros = SpendLogValue::from(moment("2024-01-02T03:04:05.123456+00:00"));
    assert_eq!(
        encoded(&with_micros),
        r#""2024-01-02T03:04:05.123456+00:00""#
    );
    let whole_seconds = SpendLogValue::from(moment("2024-01-02T03:04:05+00:00"));
    assert_eq!(encoded(&whole_seconds), r#""2024-01-02T03:04:05+00:00""#);
}

#[test]
fn a_scale_zero_decimal_keeps_a_fraction_in_its_json() {
    assert_eq!(encoded(&SpendLogValue::Decimal(dec("5"))), "5.0");
    assert_eq!(encoded(&SpendLogValue::Decimal(dec("5.25"))), "5.25");
}

#[test]
fn a_counter_keeps_its_integer_text() {
    assert_eq!(
        encoded(&SpendLogValue::Whole(i64::MAX)),
        "9223372036854775807"
    );
    assert_eq!(encoded(&SpendLogValue::Whole(-3)), "-3");
}

#[test]
fn a_nul_inside_a_text_survives_the_round_trip() {
    let row: BTreeMap<String, SpendLogValue> = decode_spend_log_row(br#"{"messages":"a\u0000b"}"#)
        .unwrap()
        .unwrap();
    assert_eq!(row["messages"], SpendLogValue::Text("a\0b".to_owned()));
    let parked = serde_json::to_string(&row).unwrap();
    assert!(
        parked.contains(r#"\u0000"#),
        "the encoder stripped the nul: {parked}"
    );
    let reread = decode_spend_log_row(parked.as_bytes()).unwrap().unwrap();
    assert_eq!(reread, row);
}

#[test]
fn a_row_keeps_its_own_kinds_and_nesting() {
    let row = decode_spend_log_row(
        br#"{"n":9223372036854775807,"spend":1.5,"ok":true,"none":null,"text":"x","list":[1,2.5],"obj":{"k":{"deep":false}}}"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(row["n"], SpendLogValue::Whole(i64::MAX));
    assert_eq!(row["spend"], SpendLogValue::Decimal(dec("1.5")));
    assert_eq!(row["ok"], SpendLogValue::Bool(true));
    assert_eq!(row["none"], SpendLogValue::Null);
    assert_eq!(row["text"], SpendLogValue::Text("x".to_owned()));
    assert_eq!(
        row["list"],
        SpendLogValue::Array(vec![
            SpendLogValue::Whole(1),
            SpendLogValue::Decimal(dec("2.5")),
        ])
    );
    assert_eq!(
        row["obj"],
        SpendLogValue::Object(BTreeMap::from([(
            "k".to_owned(),
            SpendLogValue::Object(BTreeMap::from([(
                "deep".to_owned(),
                SpendLogValue::Bool(false),
            )])),
        )]))
    );
}

#[rstest::rstest]
#[case::null(b"null")]
#[case::counter(b"1")]
#[case::text(br#""text""#)]
#[case::array(br#"[{"a":1}]"#)]
fn a_nonobject_row_decodes_to_no_row(#[case] json: &[u8]) {
    assert_eq!(decode_spend_log_row(json).unwrap(), None);
}

#[test]
fn malformed_json_is_an_error() {
    assert!(matches!(
        decode_spend_log_row(b"{\"a\":"),
        Err(SpendLogError::Json(_))
    ));
}

#[test]
fn a_counter_outside_the_i64_range_is_a_counter_error() {
    assert!(matches!(
        decode_spend_log_row(br#"{"n":9223372036854775808}"#),
        Err(SpendLogError::Counter)
    ));
}

#[test]
fn a_number_beyond_the_decimal_range_is_a_number_error() {
    assert!(matches!(
        decode_spend_log_row(br#"{"n":1e400}"#),
        Err(SpendLogError::Number(_))
    ));
}

#[test]
fn a_value_nested_past_the_depth_limit_is_too_deep() {
    let deep = format!("{{\"a\":{}1{}}}", "[".repeat(70), "]".repeat(70));
    assert!(matches!(
        decode_spend_log_row(deep.as_bytes()),
        Err(SpendLogError::TooDeep)
    ));
}
