use chrono::DateTime;
use rstest::rstest;

use crate::model_usage::UsageStartTime;
use crate::model_usage::decode::{decode_model_usage_fields, usage_date};

#[rstest]
#[case::true_value(r#"{"internal_call_origin": true}"#, true)]
#[case::non_empty_text(r#"{"internal_call_origin": "false"}"#, true)]
#[case::one(r#"{"internal_call_origin": 1}"#, true)]
#[case::negative(r#"{"internal_call_origin": -1}"#, true)]
#[case::filled_list(r#"{"internal_call_origin": [0]}"#, true)]
#[case::filled_object(r#"{"internal_call_origin": {"a": 0}}"#, true)]
#[case::false_value(r#"{"internal_call_origin": false}"#, false)]
#[case::zero(r#"{"internal_call_origin": 0}"#, false)]
#[case::zero_float(r#"{"internal_call_origin": 0.0}"#, false)]
#[case::negative_zero(r#"{"internal_call_origin": -0.0}"#, false)]
#[case::null(r#"{"internal_call_origin": null}"#, false)]
#[case::empty_text(r#"{"internal_call_origin": ""}"#, false)]
#[case::empty_list(r#"{"internal_call_origin": []}"#, false)]
#[case::empty_object(r#"{"internal_call_origin": {}}"#, false)]
#[case::absent("{}", false)]
fn internal_call_origin_uses_python_truthiness(#[case] metadata: &str, #[case] internal: bool) {
    let fields = decode_model_usage_fields(metadata.as_bytes(), b"[]");
    assert_eq!(fields.internal_call, internal);
}

#[rstest]
#[case::array("[]")]
#[case::null("null")]
#[case::text("\"internal\"")]
#[case::empty("")]
#[case::broken("{")]
fn metadata_that_is_not_an_object_is_not_internal(#[case] metadata: &str) {
    assert!(!decode_model_usage_fields(metadata.as_bytes(), b"[]").internal_call);
}

#[test]
fn metadata_that_is_not_utf8_is_not_internal() {
    assert!(!decode_model_usage_fields(b"\xff\xfe", b"[]").internal_call);
    assert_eq!(
        decode_model_usage_fields(b"\xff\xfe", b"[]").task_type,
        "uncategorized"
    );
}

#[rstest]
#[case::direct(r#"["task:code_review"]"#, "code_review")]
#[case::first_known(r#"["task:unknown","task:debugging","task:translation"]"#, "debugging")]
#[case::not_prefixed(r#"["code_review"]"#, "uncategorized")]
#[case::capitalized(r#"["task:Code_Review"]"#, "uncategorized")]
#[case::hyphenated(r#"["task:code-review"]"#, "uncategorized")]
#[case::empty_suffix(r#"["task:"]"#, "uncategorized")]
#[case::non_strings(r#"[1, null, "task:debugging"]"#, "debugging")]
#[case::no_tags("[]", "uncategorized")]
#[case::broken("[", "uncategorized")]
#[case::object(r#"{"task": "debugging"}"#, "uncategorized")]
fn the_task_type_is_the_first_known_task_tag(#[case] tags: &str, #[case] task_type: &str) {
    let fields = decode_model_usage_fields(b"{}", tags.as_bytes());
    assert_eq!(fields.task_type, task_type);
}

#[test]
fn a_text_start_time_gives_its_first_ten_characters_as_the_date() {
    assert_eq!(
        usage_date(&UsageStartTime::Text(String::from("2026-01-02T10:00:00Z"))).as_deref(),
        Some("2026-01-02")
    );
    assert_eq!(
        usage_date(&UsageStartTime::Text(String::from("2026-01-02"))).as_deref(),
        Some("2026-01-02")
    );
    assert_eq!(
        usage_date(&UsageStartTime::Text(String::from("2026-13-45"))).as_deref(),
        Some("2026-13-45")
    );
    assert_eq!(
        usage_date(&UsageStartTime::Text(String::from("2026-1-2"))),
        None
    );
    assert_eq!(usage_date(&UsageStartTime::Text(String::new())), None);
}

#[test]
fn a_text_prefix_is_counted_in_characters() {
    assert_eq!(
        usage_date(&UsageStartTime::Text(String::from(
            "\u{ff12}\u{ff10}\u{ff12}\u{ff16}-01-02T00:00:00"
        )))
        .as_deref(),
        Some("\u{ff12}\u{ff10}\u{ff12}\u{ff16}-01-02")
    );
}

#[test]
fn a_datetime_start_time_keeps_its_own_date() {
    let start = DateTime::parse_from_rfc3339("2026-01-02T23:30:00-05:00").expect("a test stamp");
    assert_eq!(
        usage_date(&UsageStartTime::Parsed(start)).as_deref(),
        Some("2026-01-02")
    );
}
