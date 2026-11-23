#![cfg(test)]

mod resolve;
mod routes;
mod summary;
mod support;

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use rstest::rstest;

use self::support::json;
use super::content::{has_content, holds_truncation, request_snapshot};
use super::dates::{DetailDateError, decode_detail_dates};
use super::metadata::{cold_storage_object_key, decode_log_metadata, hydrate_log_metadata};

fn decoded(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::null("null", false)]
#[case::empty_text(r#""""#, false)]
#[case::empty_object_text(r#""{}""#, false)]
#[case::empty_list_text(r#""[]""#, false)]
#[case::null_text(r#""null""#, false)]
#[case::padded_empty_object_text(r#"" \n {} \t""#, false)]
#[case::separator_padded_list_text(r#""\u001c[]\u001f""#, false)]
#[case::blank_text(r#""   ""#, false)]
#[case::content_text(r#""hello""#, true)]
#[case::nested_empty_text(r#""[{}]""#, true)]
#[case::empty_list("[]", false)]
#[case::empty_object("{}", false)]
#[case::list_of_empty("[{}]", true)]
#[case::object("{\"a\":null}", true)]
#[case::zero("0", true)]
#[case::false_flag("false", true)]
fn spend_log_field_content_follows_the_stored_value(#[case] stored: &str, #[case] expected: bool) {
    assert_eq!(has_content(Some(&decoded(stored))), expected);
}

#[test]
fn a_missing_spend_log_field_has_no_content() {
    assert!(!has_content(None));
}

#[rstest]
#[case::marker_text(r#""x litellm_truncated y""#, true)]
#[case::marker_key(r#"{"litellm_truncated_chars":1}"#, true)]
#[case::nested_marker(
    r#"{"body":{"messages":[{"content":"... (litellm_truncated skipped 9 chars) ..."}]}}"#,
    true
)]
#[case::plain(r#"{"body":{"messages":["hi"]}}"#, false)]
#[case::split_marker(r#"{"litellm_":"truncated"}"#, false)]
#[case::number("12", false)]
fn truncation_is_found_in_any_key_or_string(#[case] stored: &str, #[case] expected: bool) {
    assert_eq!(holds_truncation(&decoded(stored)), expected);
}

#[rstest]
#[case::object(r#"{"model":"m"}"#, Some(r#"{"model":"m"}"#))]
#[case::object_text(r#""{\"model\":\"m\"}""#, Some(r#"{"model":"m"}"#))]
#[case::list_text(r#""[1]""#, None)]
#[case::malformed_text(r#""{nope""#, None)]
#[case::list("[1]", None)]
#[case::null("null", None)]
fn a_request_snapshot_is_a_json_object(#[case] stored: &str, #[case] expected: Option<&str>) {
    let snapshot = request_snapshot(&decoded(stored)).map(Json::Object);
    assert_eq!(snapshot, expected.map(decoded));
}

#[rstest]
#[case::object_text(r#""{\"a\":1}""#, r#"{"a":1}"#)]
#[case::number_text(r#""5""#, "5")]
#[case::string_text(r#""\"text\"""#, r#""text""#)]
#[case::null_text(r#""null""#, "null")]
#[case::list_text(r#""[1,2]""#, "[1,2]")]
#[case::malformed_text(r#""not json""#, "{}")]
#[case::empty_text(r#""""#, "{}")]
#[case::object(r#"{"a":"{\"b\":1}"}"#, r#"{"a":"{\"b\":1}"}"#)]
#[case::number("7", "7")]
#[case::null("null", "null")]
fn hydrates_string_metadata_and_keeps_the_rest(#[case] stored: &str, #[case] expected: &str) {
    let mut metadata = json(stored);
    hydrate_log_metadata(&mut metadata);
    assert_eq!(metadata, json(expected));
}

#[test]
fn a_repeated_metadata_key_keeps_its_last_value() {
    assert_eq!(decode_log_metadata(br#"{"a":1,"a":2}"#), json(r#"{"a":2}"#));
}

#[rstest]
#[case::object(
    r#"{"cold_storage_object_key":"2025/01/01/req.json"}"#,
    Some("2025/01/01/req.json")
)]
#[case::object_text(r#""{\"cold_storage_object_key\":\"k1\"}""#, Some("k1"))]
#[case::empty_key(r#"{"cold_storage_object_key":""}"#, None)]
#[case::number_key(r#"{"cold_storage_object_key":3}"#, None)]
#[case::null_key(r#"{"cold_storage_object_key":null}"#, None)]
#[case::missing_key(r#"{"other":"k1"}"#, None)]
#[case::malformed_text(r#""{cold_storage_object_key""#, None)]
#[case::scalar_text(r#""\"k1\"""#, None)]
#[case::list(r#"[{"cold_storage_object_key":"k1"}]"#, None)]
#[case::null("null", None)]
fn reads_the_cold_storage_key_from_metadata(#[case] stored: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        cold_storage_object_key(Some(&json(stored))).as_deref(),
        expected
    );
}

#[test]
fn a_missing_metadata_column_names_no_cold_key() {
    assert_eq!(cold_storage_object_key(None), None);
}

#[rstest]
#[case::both(
    Some("2025-01-02 03:04:05"),
    Some("2025-01-03 00:00:00"),
    Some("2025-01-02T03:04:05+00:00"),
    Some("2025-01-03T00:00:00+00:00")
)]
#[case::start_only(
    Some("2025-01-02 03:04:05"),
    None,
    Some("2025-01-02T03:04:05+00:00"),
    None
)]
#[case::end_only(
    None,
    Some("2024-02-29 23:59:59"),
    None,
    Some("2024-02-29T23:59:59+00:00")
)]
#[case::neither(None, None, None, None)]
fn reads_detail_dates_as_utc(
    #[case] start: Option<&str>,
    #[case] end: Option<&str>,
    #[case] start_utc: Option<&str>,
    #[case] end_utc: Option<&str>,
) {
    let window = decode_detail_dates(start, end).unwrap();
    assert_eq!(
        window
            .start_time_utc
            .map(|time| time.to_rfc3339())
            .as_deref(),
        start_utc
    );
    assert_eq!(
        window.end_time_utc.map(|time| time.to_rfc3339()).as_deref(),
        end_utc
    );
}

#[rstest]
#[case::date_only(Some("2025-01-02"), None, "start_date")]
#[case::iso_t(Some("2025-01-02T03:04:05"), None, "start_date")]
#[case::trailing_text(None, Some("2025-01-02 03:04:05Z"), "end_date")]
#[case::empty(None, Some(""), "end_date")]
#[case::impossible_day(Some("2025-02-30 00:00:00"), None, "start_date")]
#[case::both_bad(Some("bad"), Some("worse"), "start_date")]
fn rejects_a_detail_date_outside_the_format(
    #[case] start: Option<&str>,
    #[case] end: Option<&str>,
    #[case] named: &str,
) {
    let DetailDateError::Format { field, .. } = decode_detail_dates(start, end).unwrap_err();
    assert_eq!(field, named);
}

// each corpus file names its parser on the first line: metadata text, or a start and an end date
#[rstest]
fn replays_the_log_metadata_corpus(#[files("fuzz/corpus/log_metadata/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let mut parts = bytes.splitn(2, |byte| *byte == b'\n');
    let tag = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    match tag {
        b"metadata" => {
            let metadata = decode_log_metadata(rest);
            let again = decode_log_metadata(&serde_json::to_vec(&metadata).unwrap());
            assert_eq!(again, metadata);
            if let Ok(text) = from_utf8(rest) {
                let mut stored = JsonData::String(text.to_owned());
                hydrate_log_metadata(&mut stored);
                assert_eq!(stored, metadata);
                drop(cold_storage_object_key(Some(&stored)));
            }
        }
        b"dates" => {
            let text = from_utf8(rest).unwrap();
            let mut lines = text.splitn(2, '\n');
            let start = lines.next().filter(|line| !line.is_empty());
            let end = lines.next();
            drop(decode_detail_dates(start, end));
        }
        other => panic!("unknown corpus tag {other:?}"),
    }
}
