use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use rstest::rstest;
use serde_json::Number;

use crate::model_info_search::datetime::{
    decode_model_info_datetime, normalize_model_info_datetime,
};

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

#[test]
fn normalizes_every_named_iso_form() {
    for (input, expected) in [
        ("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"),
        ("2026-01-01Z00:00:00", "2026-01-01T00:00:00Z"),
        ("2026-01-01Z", "2026-01-01T00:00:00Z"),
        ("20260101Z000000", "2026-01-01T00:00:00Z"),
        ("2026-01-01T00:00:00+05:30", "2025-12-31T18:30:00Z"),
        ("2026-01-01T00:00:00-05:00", "2026-01-01T05:00:00Z"),
        ("2026-01-01T00:00:00", "2026-01-01T00:00:00Z"),
        ("2026-01-01", "2026-01-01T00:00:00Z"),
        ("2026-W01-1T00:00:00", "2025-12-29T00:00:00Z"),
    ] {
        assert_eq!(decode_model_info_datetime(input), Some(utc(expected)));
    }
    assert_eq!(
        decode_model_info_datetime("2026-01-01T00:00:00.1234567")
            .map(|parsed| parsed.timestamp_subsec_micros()),
        Some(123_456)
    );
    let max = decode_model_info_datetime("9999-12-31T23:59:59.999999Z").expect("the last instant");
    assert_eq!(max.timestamp_subsec_micros(), 999_999);
    assert_eq!(max.date_naive().to_string(), "9999-12-31");
}

#[rstest]
#[case::empty("")]
#[case::double_z("2026-01-01T00:00:00ZZ")]
#[case::leading_space(" 2026-01-01T00:00:00")]
#[case::trailing_space("2026-01-01T00:00:00 ")]
#[case::impossible_day("2026-02-30T00:00:00")]
#[case::z_then_offset("2026-01-01T00:00:00Z+05:00")]
#[case::year_zero("0000-01-01T00:00:00")]
#[case::lowercase_z("2026-01-01T00:00:00z")]
#[case::double_separator("2026-01-01T00:00:00T00:00:00")]
#[case::unicode_digits(
    "\u{662}\u{660}\u{662}\u{666}-\u{660}\u{661}-\u{660}\u{661}T\u{660}\u{660}:\u{660}\u{660}:\u{660}\u{660}"
)]
#[case::comma_fraction_end("2026-01-01T00:00:00,")]
fn rejects_invalid_text(#[case] text: &str) {
    assert_eq!(decode_model_info_datetime(text), None);
}

#[rstest]
#[case::null(Json::Null)]
#[case::flag(Json::Bool(true))]
#[case::number(Json::Number(Number::from(5_u8)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(JsonObject::new()))]
fn rejects_other_json_types(#[case] value: Json) {
    assert_eq!(normalize_model_info_datetime(&value), None);
}

#[test]
fn normalizes_a_text_field_through_the_string_decoder() {
    let value = Json::Text("2026-01-01T00:00:00Z".to_owned());
    assert_eq!(
        normalize_model_info_datetime(&value),
        Some(utc("2026-01-01T00:00:00Z"))
    );
}
