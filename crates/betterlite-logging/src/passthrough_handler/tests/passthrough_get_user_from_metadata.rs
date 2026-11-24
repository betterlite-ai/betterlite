use rstest::rstest;

use super::support::{harness, json_object, logging_payload};

#[rstest]
#[case::user_field(r#"{"user": "alice"}"#, Some("alice"))]
#[case::user_is_trimmed(r#"{"user": "  alice  "}"#, Some("alice"))]
#[case::metadata_user_id(r#"{"metadata": {"user_id": "bob"}}"#, Some("bob"))]
#[case::litellm_metadata_user(r#"{"litellm_metadata": {"user": "carol"}}"#, Some("carol"))]
#[case::numeric_user(r#"{"user": 42}"#, Some("42"))]
#[case::empty_user(r#"{"user": ""}"#, None)]
#[case::null_user(r#"{"user": null}"#, None)]
#[case::no_user_field(r#"{"max_tokens": 16}"#, None)]
fn reads_the_end_user_from_the_request_body(#[case] body: &str, #[case] expected: Option<&str>) {
    let harness = harness();
    let payload = logging_payload(Some(json_object(body)));
    assert_eq!(
        harness.handler.get_user_from_metadata(&payload).as_deref(),
        expected
    );
}

#[test]
fn returns_none_without_a_request_body() {
    let harness = harness();
    assert_eq!(
        harness
            .handler
            .get_user_from_metadata(&logging_payload(None)),
        None
    );
}

#[test]
fn the_extraction_settings_come_from_the_handler() {
    let mut harness = harness();
    let payload = logging_payload(Some(json_object(r#"{"user": "{\"id\": 7}"}"#)));
    harness.handler.validate_end_user_id_in_db = true;
    assert_eq!(harness.handler.get_user_from_metadata(&payload), None);
    harness.handler.validate_end_user_id_in_db = false;
    assert_eq!(
        harness.handler.get_user_from_metadata(&payload).as_deref(),
        Some(r#"{"id": 7}"#)
    );
}
