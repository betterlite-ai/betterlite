#![cfg(test)]

use betterlite_config::sections::request_metadata_safety::MissingSessionIdPolicy;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::metadata::MetadataField;
use super::super::session::apply_missing_session_id_policy;
use super::super::{MetadataSafetySettings, sanitize_request_metadata};
use super::{body, field, input, object, settings, text};
use crate::api::api_boundaries::ApiLimits;

#[test]
fn true_redaction_opt_outs_are_kept() {
    let data = body(vec![
        ("turn_off_message_logging", Json::Bool(true)),
        (
            "metadata",
            object(vec![("turn_off_message_logging", Json::Bool(true))]),
        ),
    ]);
    let result = sanitize_request_metadata(&input(data, "/v1/chat/completions"), &settings())
        .expect("the slots are objects");
    assert_eq!(
        result.data.get("turn_off_message_logging"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        field(&result.data, "metadata").get("turn_off_message_logging"),
        Some(&Json::Bool(true))
    );
}

#[rstest]
#[case::fits("{\"a\":\"bc\"}", true)]
#[case::one_byte_over("{\"a\":\"bcd\"}", false)]
fn metadata_string_length_boundary(#[case] raw: &str, #[case] accepted: bool) {
    let bounded = MetadataSafetySettings {
        limits: ApiLimits {
            max_json_bytes: 10,
            max_event_bytes: 1024 * 1024,
            max_items: 64 * 1024,
        },
        missing_session_id: None,
        turn_off_message_logging: settings().turn_off_message_logging,
    };
    let data = body(vec![("metadata", text(raw))]);
    let outcome = sanitize_request_metadata(&input(data, "/v1/chat/completions"), &bounded);
    assert_eq!(outcome.is_ok(), accepted);
}

#[test]
fn null_slot_session_still_copies_the_body_session() {
    let mut data = body(vec![
        ("litellm_session_id", text("body-session")),
        ("metadata", object(vec![])),
        ("litellm_metadata", object(vec![("session_id", Json::Null)])),
    ]);
    let policy = MissingSessionIdPolicy::Omit;
    apply_missing_session_id_policy(
        &mut data,
        MetadataField::LitellmMetadata,
        Some(&policy),
        true,
    );
    assert_eq!(
        field(&data, "litellm_metadata").get("session_id"),
        Some(&text("body-session"))
    );
}
