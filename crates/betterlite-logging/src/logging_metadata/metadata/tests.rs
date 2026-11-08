#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::get_standard_logging_metadata;
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::decode::DecodeError;

fn metadata(encoded: &str) -> IndexMap<String, Json> {
    serde_json::from_str(encoded).expect("metadata decodes")
}

#[test]
fn a_missing_record_has_every_field_absent() {
    let record = get_standard_logging_metadata(None).expect("record");
    assert_eq!(record, StandardLoggingMetadata::default());
}

#[rstest]
#[case::oauth_token(r#"{"used_client_oauth_token": true}"#)]
#[case::team_alias(r#"{"team_alias": "team"}"#)]
#[case::cold_storage(r#"{"cold_storage_object_key": "key"}"#)]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str) {
    let record = get_standard_logging_metadata(Some(&metadata(encoded))).expect("record");
    assert_ne!(record, StandardLoggingMetadata::default());
}

#[test]
fn the_module_level_copy_includes_used_client_oauth_token() {
    let record =
        get_standard_logging_metadata(Some(&metadata(r#"{"used_client_oauth_token": false}"#)))
            .expect("record");
    assert_eq!(record.used_client_oauth_token, Some(false));
}

#[test]
fn the_module_level_copy_keeps_only_declared_fields() {
    let record =
        get_standard_logging_metadata(Some(&metadata(r#"{"user_agent": "agent", "unknown": 1}"#)))
            .expect("record");
    assert_eq!(record.user_agent.as_deref(), Some("agent"));
    let encoded = serde_json::to_string(&record).expect("record encodes");
    assert!(!encoded.contains("unknown"));
}

#[test]
fn the_module_level_copy_takes_a_valid_user_api_key_as_the_hash() {
    let digest = "B".repeat(64);
    let record = get_standard_logging_metadata(Some(&metadata(&format!(
        r#"{{"user_api_key": "{digest}"}}"#
    ))))
    .expect("record");
    assert_eq!(record.user_api_key_hash, Some(digest));
}

#[test]
fn the_module_level_copy_leaves_a_raw_key_out_of_the_hash_field() {
    let record = get_standard_logging_metadata(Some(&metadata(r#"{"user_api_key": "sk-raw"}"#)))
        .expect("record");
    assert!(record.user_api_key_hash.is_none());
}

#[test]
fn the_module_level_copy_keeps_a_recorded_hash() {
    let record =
        get_standard_logging_metadata(Some(&metadata(r#"{"user_api_key_hash": "recorded"}"#)))
            .expect("record");
    assert_eq!(record.user_api_key_hash.as_deref(), Some("recorded"));
}

#[test]
fn a_mistyped_field_is_reported_by_name() {
    let error = get_standard_logging_metadata(Some(&metadata(r#"{"user_api_key_team_id": 9}"#)))
        .expect_err("wrong type");
    let DecodeError::Field { path, record, .. } = error else {
        panic!("a field error is expected");
    };
    assert_eq!(path, "user_api_key_team_id");
    assert_eq!(record, "standard logging metadata");
}

#[rstest]
#[case::lowercase("a".repeat(64), true)]
#[case::uppercase("B".repeat(64), true)]
#[case::mixed(format!("{}{}", "aF09".repeat(8), "5e7C".repeat(8)), true)]
#[case::too_short("a".repeat(63), false)]
#[case::too_long("a".repeat(65), false)]
#[case::empty(String::new(), false)]
#[case::not_hex("g".repeat(64), false)]
#[case::raw_key(format!("sk-{}", "abcdefghijklmnopqrstuvwxyz0123456789".repeat(2)), false)]
fn only_a_sha256_shaped_user_api_key_becomes_the_hash(
    #[case] user_api_key: String,
    #[case] is_hash: bool,
) {
    let encoded = format!(r#"{{"user_api_key": "{user_api_key}"}}"#);
    let record = get_standard_logging_metadata(Some(&metadata(&encoded))).expect("record");
    let expected = is_hash.then_some(user_api_key);
    assert_eq!(record.user_api_key_hash, expected);
}
