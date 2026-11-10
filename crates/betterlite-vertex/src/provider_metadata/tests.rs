#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use jiff::tz::TimeZone;
use rstest::rstest;

use crate::provider_metadata::redaction::{
    VERTEX_METADATA_FIELDS, redact_litellm_params_metadata, redact_logged_object_metadata,
};
use crate::provider_metadata::timestamp::{
    LocalTimeContext, VertexTimestampError, decode_vertex_timestamp,
};

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

enum MetadataCase {
    LoggedObject {
        input: &'static str,
        expected: &'static str,
    },
    CallDetails {
        input: &'static str,
        expected: &'static str,
    },
    Timestamp {
        zone: &'static str,
        text: &'static str,
        expected: i64,
    },
    InvalidTimestamp {
        zone: &'static str,
        text: &'static str,
    },
}

#[rstest]
#[case::redacts_all_provider_fields(MetadataCase::LoggedObject {
    input: r#"{"vertex_ai_grounding_metadata": [1], "vertex_ai_url_context_metadata": [2], "vertex_ai_safety_ratings": [3], "vertex_ai_safety_results": [4], "vertex_ai_citation_metadata": [5], "_hidden_params": {"vertex_ai_grounding_metadata": [1], "vertex_ai_citation_metadata": [5], "kept": true}, "keep": true}"#,
    expected: r#"{"vertex_ai_grounding_metadata": [], "vertex_ai_url_context_metadata": [], "vertex_ai_safety_ratings": [], "vertex_ai_safety_results": [], "vertex_ai_citation_metadata": [], "_hidden_params": {"kept": true}, "keep": true}"#,
})]
#[case::object_without_provider_fields(MetadataCase::LoggedObject {
    input: r#"{"keep": 1}"#,
    expected: r#"{"keep": 1}"#,
})]
#[case::nested_provider_fields_are_not_redacted(MetadataCase::LoggedObject {
    input: r#"{"nested": {"vertex_ai_grounding_metadata": [1]}}"#,
    expected: r#"{"nested": {"vertex_ai_grounding_metadata": [1]}}"#,
})]
#[case::hidden_params_not_a_map(MetadataCase::LoggedObject {
    input: r#"{"vertex_ai_grounding_metadata": null, "_hidden_params": "copies"}"#,
    expected: r#"{"vertex_ai_grounding_metadata": [], "_hidden_params": "copies"}"#,
})]
#[case::logged_string_is_untouched(MetadataCase::LoggedObject {
    input: r#""not an object""#,
    expected: r#""not an object""#,
})]
#[case::logged_list_is_untouched(MetadataCase::LoggedObject {
    input: "[1, 2]",
    expected: "[1, 2]",
})]
#[case::redacts_both_metadata_copies(MetadataCase::CallDetails {
    input: r#"{"litellm_params": {"metadata": {"hidden_params": {"vertex_ai_grounding_metadata": [1], "vertex_ai_safety_results": [4], "kept": 2}}, "litellm_metadata": {"hidden_params": {"vertex_ai_citation_metadata": [5]}}}}"#,
    expected: r#"{"litellm_params": {"metadata": {"hidden_params": {"kept": 2}}, "litellm_metadata": {"hidden_params": {}}}}"#,
})]
#[case::call_details_hidden_params_not_a_map(MetadataCase::CallDetails {
    input: r#"{"litellm_params": {"metadata": {"hidden_params": "copies"}}}"#,
    expected: r#"{"litellm_params": {"metadata": {"hidden_params": "copies"}}}"#,
})]
#[case::call_details_metadata_not_a_map(MetadataCase::CallDetails {
    input: r#"{"litellm_params": {"litellm_metadata": "copies"}}"#,
    expected: r#"{"litellm_params": {"litellm_metadata": "copies"}}"#,
})]
#[case::call_details_litellm_params_not_a_map(MetadataCase::CallDetails {
    input: r#"{"litellm_params": "copies"}"#,
    expected: r#"{"litellm_params": "copies"}"#,
})]
#[case::call_details_without_litellm_params(MetadataCase::CallDetails {
    input: r#"{"other": 1}"#,
    expected: r#"{"other": 1}"#,
})]
#[case::call_details_null_litellm_params(MetadataCase::CallDetails {
    input: r#"{"litellm_params": null}"#,
    expected: r#"{"litellm_params": null}"#,
})]
#[case::call_details_non_object(MetadataCase::CallDetails {
    input: "[1]",
    expected: "[1]",
})]
#[case::utc_whole_second(MetadataCase::Timestamp { zone: "UTC", text: "2024-07-01T12:00:00.000000Z", expected: 1_719_835_200 })]
#[case::utc_tenth(MetadataCase::Timestamp { zone: "UTC", text: "2024-01-02T03:04:05.5Z", expected: 1_704_164_645 })]
#[case::utc_microsecond_truncates(MetadataCase::Timestamp { zone: "UTC", text: "2024-01-02T03:04:05.000001Z", expected: 1_704_164_645 })]
#[case::utc_last_microsecond(MetadataCase::Timestamp { zone: "UTC", text: "2024-01-02T03:04:05.999999Z", expected: 1_704_164_645 })]
#[case::single_digit_fields(MetadataCase::Timestamp { zone: "UTC", text: "2024-1-2T3:4:5.5Z", expected: 1_704_164_645 })]
#[case::lowercase_separators(MetadataCase::Timestamp { zone: "UTC", text: "2024-01-02t03:04:05.5z", expected: 1_704_164_645 })]
#[case::space_padded_day(MetadataCase::Timestamp { zone: "UTC", text: "2024-01- 2T03:04:05.5Z", expected: 1_704_164_645 })]
#[case::space_padded_hour(MetadataCase::Timestamp { zone: "UTC", text: "2024-01-02T 3:04:05.5Z", expected: 1_704_164_645 })]
#[case::space_padded_day_and_hour(MetadataCase::Timestamp { zone: "UTC", text: "2024-01- 9T 9:04:05.5Z", expected: 1_704_791_045 })]
#[case::first_century(MetadataCase::Timestamp { zone: "UTC", text: "0001-01-02T03:04:05.5Z", expected: -62_135_499_354 })]
#[case::last_century_whole_second(MetadataCase::Timestamp { zone: "UTC", text: "9999-12-31T23:59:59.000000Z", expected: 253_402_300_799 })]
#[case::last_century_fraction_truncates(MetadataCase::Timestamp { zone: "UTC", text: "9999-12-31T23:59:59.999999Z", expected: 253_402_300_799 })]
#[case::last_century_zone_offset(MetadataCase::Timestamp { zone: "America/New_York", text: "9999-12-31T23:59:59.000000Z", expected: 253_402_318_799 })]
#[case::tokyo_year_end_neighbor(MetadataCase::Timestamp { zone: "Asia/Tokyo", text: "9999-12-31T10:00:00.000000Z", expected: 253_402_218_000 })]
#[case::tokyo_year_end_last_valid(MetadataCase::Timestamp { zone: "Asia/Tokyo", text: "9999-12-31T14:59:59.000000Z", expected: 253_402_235_999 })]
#[case::tokyo_year_end_is_rejected(MetadataCase::InvalidTimestamp { zone: "Asia/Tokyo", text: "9999-12-31T23:59:59.000000Z" })]
#[case::first_day_of_year_one_is_rejected(MetadataCase::InvalidTimestamp { zone: "UTC", text: "0001-01-01T00:00:00.000000Z" })]
#[case::first_day_of_year_one_end_is_rejected(MetadataCase::InvalidTimestamp { zone: "UTC", text: "0001-01-01T23:59:59.000000Z" })]
#[case::first_day_of_year_one_west_zone_is_rejected(MetadataCase::InvalidTimestamp { zone: "America/New_York", text: "0001-01-01T00:00:00.000000Z" })]
#[case::new_york_summer(MetadataCase::Timestamp { zone: "America/New_York", text: "2024-07-01T12:00:00.000000Z", expected: 1_719_849_600 })]
#[case::new_york_fold_takes_the_earlier_offset(MetadataCase::Timestamp { zone: "America/New_York", text: "2024-11-03T01:30:00.000000Z", expected: 1_730_611_800 })]
#[case::new_york_gap_takes_the_before_offset(MetadataCase::Timestamp { zone: "America/New_York", text: "2024-03-10T02:30:00.000000Z", expected: 1_710_055_800 })]
#[case::berlin_gap(MetadataCase::Timestamp { zone: "Europe/Berlin", text: "2024-03-31T02:30:00.000000Z", expected: 1_711_848_600 })]
#[case::berlin_fold(MetadataCase::Timestamp { zone: "Europe/Berlin", text: "2024-10-27T02:30:00.000000Z", expected: 1_729_989_000 })]
#[case::epoch_subsecond(MetadataCase::Timestamp { zone: "UTC", text: "1970-01-01T00:00:00.999999Z", expected: 0 })]
#[case::negative_half_truncates_toward_zero(MetadataCase::Timestamp { zone: "UTC", text: "1969-12-31T23:59:59.500000Z", expected: 0 })]
#[case::negative_whole_second_stays_negative(MetadataCase::Timestamp { zone: "UTC", text: "1969-12-31T23:59:59.000000Z", expected: -1 })]
#[case::negative_microsecond_truncates_toward_zero(MetadataCase::Timestamp { zone: "UTC", text: "1969-12-31T23:59:59.999999Z", expected: 0 })]
#[case::negative_one_and_a_half(MetadataCase::Timestamp { zone: "UTC", text: "1969-12-31T23:59:58.500000Z", expected: -1 })]
#[case::missing_zone_marker(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:05.5" })]
#[case::empty_fraction(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:05.Z" })]
#[case::seven_fraction_digits(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:05.1234567Z" })]
#[case::year_zero(MetadataCase::InvalidTimestamp { zone: "UTC", text: "0000-01-02T03:04:05.5Z" })]
#[case::short_year(MetadataCase::InvalidTimestamp { zone: "UTC", text: "24-01-02T03:04:05.5Z" })]
#[case::month_thirteen(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-13-02T03:04:05.5Z" })]
#[case::month_zero(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-00-02T03:04:05.5Z" })]
#[case::day_zero(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-00T03:04:05.5Z" })]
#[case::february_thirty(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-02-30T03:04:05.5Z" })]
#[case::hour_twenty_four(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T24:04:05.5Z" })]
#[case::minute_sixty(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:60:05.5Z" })]
#[case::second_sixty(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:60.5Z" })]
#[case::trailing_character(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:05.5ZZ" })]
#[case::leading_space(MetadataCase::InvalidTimestamp { zone: "UTC", text: " 2024-01-02T03:04:05.5Z" })]
#[case::trailing_space(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:05.5Z " })]
#[case::extra_date_field(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02-03T03:04:05.5Z" })]
#[case::extra_time_field(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03:04:05:06.5Z" })]
#[case::space_separator(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02 03:04:05.5Z" })]
#[case::space_padded_day_zero(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01- 0T03:04:05.5Z" })]
#[case::space_padded_two_digit_day(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01- 12T03:04:05.5Z" })]
#[case::space_padded_two_digit_hour(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T 12:04:05.5Z" })]
#[case::space_padded_minute(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-01-02T03: 4:05.5Z" })]
#[case::space_padded_month(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024- 1-02T03:04:05.5Z" })]
#[case::three_digit_month(MetadataCase::InvalidTimestamp { zone: "UTC", text: "2024-001-02T03:04:05.5Z" })]
#[case::empty_text(MetadataCase::InvalidTimestamp { zone: "UTC", text: "" })]
fn metadata_cases(#[case] case: MetadataCase) {
    match case {
        MetadataCase::LoggedObject { input, expected } => {
            let mut object = json(input);
            redact_logged_object_metadata(&mut object);
            assert_eq!(object, json(expected));
        }
        MetadataCase::CallDetails { input, expected } => {
            let mut details = json(input);
            redact_litellm_params_metadata(&mut details);
            assert_eq!(details, json(expected));
        }
        MetadataCase::Timestamp {
            zone,
            text,
            expected,
        } => {
            let context = LocalTimeContext {
                zone: TimeZone::get(zone).unwrap(),
            };
            assert_eq!(
                decode_vertex_timestamp(text.as_bytes(), &context).ok(),
                Some(expected),
                "zone {zone} text {text}"
            );
        }
        MetadataCase::InvalidTimestamp { zone, text } => {
            let context = LocalTimeContext {
                zone: TimeZone::get(zone).unwrap(),
            };
            assert!(
                decode_vertex_timestamp(text.as_bytes(), &context).is_err(),
                "zone {zone} text {text}"
            );
        }
    }
}

#[test]
fn provider_metadata_fields_are_the_five_wire_names() {
    assert_eq!(
        VERTEX_METADATA_FIELDS,
        [
            "vertex_ai_grounding_metadata",
            "vertex_ai_url_context_metadata",
            "vertex_ai_safety_ratings",
            "vertex_ai_safety_results",
            "vertex_ai_citation_metadata",
        ]
    );
}

#[test]
fn non_utf8_timestamp_is_rejected() {
    let context = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    assert!(matches!(
        decode_vertex_timestamp(&[0xff, 0xfe], &context),
        Err(VertexTimestampError::Utf8(_))
    ));
}

#[test]
fn timestamp_byte_limit_is_exact() {
    let context = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let at_limit = vec![b'0'; 64];
    assert!(matches!(
        decode_vertex_timestamp(&at_limit, &context),
        Err(VertexTimestampError::Format)
    ));
    let over_limit = vec![b'0'; 65];
    assert!(matches!(
        decode_vertex_timestamp(&over_limit, &context),
        Err(VertexTimestampError::TooLong {
            size: 65,
            limit: 64
        })
    ));
}

#[rstest]
fn replays_the_vertex_timestamp_corpus(#[files("fuzz/corpus/vertex_timestamp/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let context = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _decoded = decode_vertex_timestamp(&bytes, &context);
}
