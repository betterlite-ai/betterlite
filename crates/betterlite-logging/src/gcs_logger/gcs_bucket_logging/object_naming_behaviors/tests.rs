#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::tests::support::{Answer, metadata, open, payload};
use super::{ObjectNameError, get_object_name};

const FIRST_ID: &str = "00000000000000000000000000000000";
const SECOND_ID: &str = "00000000000000000000000000000001";

#[tokio::test]
async fn a_success_payload_is_named_after_the_response_id() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", None),
        "chatcmpl-1",
        &IndexMap::new(),
    )
    .expect("the name builds");
    assert_eq!(name, "2024-03-09/chatcmpl-1");
}

#[tokio::test]
async fn an_empty_response_id_leaves_an_empty_last_segment() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", None),
        "",
        &IndexMap::new(),
    )
    .expect("the name builds");
    assert_eq!(name, "2024-03-09/");
}

#[tokio::test]
async fn a_failure_payload_is_named_after_a_fresh_uuid() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", Some("the upstream call failed")),
        "",
        &IndexMap::new(),
    )
    .expect("the name builds");
    assert_eq!(name, format!("2024-03-09/failure-{FIRST_ID}"));
}

#[tokio::test]
async fn a_custom_log_id_overrides_the_name() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let litellm_metadata = metadata("gcs_log_id", Json::Text("my run/bad id.txt".to_owned()));
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", None),
        "chatcmpl-1",
        &litellm_metadata,
    )
    .expect("the name builds");
    assert_eq!(name, format!("2024-03-09/custom-{FIRST_ID}-bad_id.txt"));
}

#[tokio::test]
async fn a_failure_with_a_custom_log_id_uses_the_custom_name() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let litellm_metadata = metadata("gcs_log_id", Json::Text("my-run".to_owned()));
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", Some("the upstream call failed")),
        "",
        &litellm_metadata,
    )
    .expect("the name builds");
    assert_eq!(name, format!("2024-03-09/custom-{SECOND_ID}-my-run"));
}

#[rstest]
#[case::dots("...")]
#[case::slash("/")]
#[case::spaced_slash(" / ")]
#[tokio::test]
async fn a_custom_log_id_that_sanitizes_to_nothing_keeps_the_standard_name(#[case] empty: &str) {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let litellm_metadata = metadata("gcs_log_id", Json::Text(empty.to_owned()));
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", None),
        "chatcmpl-1",
        &litellm_metadata,
    )
    .expect("the name builds");
    assert_eq!(name, "2024-03-09/chatcmpl-1");
}

#[tokio::test]
async fn a_null_custom_log_id_keeps_the_standard_name() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let litellm_metadata = metadata("gcs_log_id", Json::Null);
    let name = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", None),
        "chatcmpl-1",
        &litellm_metadata,
    )
    .expect("the name builds");
    assert_eq!(name, "2024-03-09/chatcmpl-1");
}

#[tokio::test]
async fn a_non_string_custom_log_id_is_refused() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let litellm_metadata = metadata("gcs_log_id", Json::Bool(true));
    let error = get_object_name(
        &harness.logger,
        &payload("chatcmpl-1", None),
        "chatcmpl-1",
        &litellm_metadata,
    )
    .expect_err("the log id must be a string");
    assert!(matches!(error, ObjectNameError::GcsLogId));
}
