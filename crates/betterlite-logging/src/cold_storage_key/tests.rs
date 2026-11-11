#![cfg(test)]

use chrono::{NaiveDate, NaiveDateTime};
use rstest::rstest;

use crate::cold_storage_key::{ColdStorageTarget, stamp_cold_storage_object_key};
use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::metadata::get_standard_logging_metadata;
use crate::logging_metadata::params::MetadataMap;
use crate::s3_transport::options::PartitionGranularity;

fn start_time() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 1, 2)
        .expect("a valid date")
        .and_hms_micro_opt(3, 4, 5, 123_456)
        .expect("a valid time")
}

fn target(s3_path: &str, partition_granularity: PartitionGranularity) -> ColdStorageTarget {
    ColdStorageTarget {
        s3_path: s3_path.to_owned(),
        partition_granularity,
    }
}

fn record(encoded: &str) -> StandardLoggingMetadata {
    let metadata: MetadataMap = serde_json::from_str(encoded).expect("metadata decodes");
    get_standard_logging_metadata(Some(&metadata)).expect("metadata record decodes")
}

#[rstest]
#[case::no_path(
    "",
    PartitionGranularity::Day,
    "chatcmpl-1",
    "2026-01-02/time-03-04-05-123456_chatcmpl-1.json"
)]
#[case::day_partitions(
    "logs/",
    PartitionGranularity::Day,
    "chatcmpl-1",
    "logs/2026-01-02/time-03-04-05-123456_chatcmpl-1.json"
)]
#[case::hour_partitions(
    "logs/",
    PartitionGranularity::Hour,
    "chatcmpl-1",
    "logs/2026-01-02/03/time-03-04-05-123456_chatcmpl-1.json"
)]
#[case::path_without_a_slash(
    "logs",
    PartitionGranularity::Day,
    "chatcmpl-1",
    "logs/2026-01-02/time-03-04-05-123456_chatcmpl-1.json"
)]
#[case::two_trailing_slashes(
    "logs//",
    PartitionGranularity::Day,
    "chatcmpl-1",
    "logs/2026-01-02/time-03-04-05-123456_chatcmpl-1.json"
)]
#[case::nested_path(
    "team/cold",
    PartitionGranularity::Day,
    "chatcmpl-1",
    "team/cold/2026-01-02/time-03-04-05-123456_chatcmpl-1.json"
)]
#[case::slash_and_colon_in_id(
    "logs/",
    PartitionGranularity::Day,
    "a/b:c",
    "logs/2026-01-02/time-03-04-05-123456_a_b_c.json"
)]
fn the_stamp_writes_the_litellm_key(
    #[case] s3_path: &str,
    #[case] partition_granularity: PartitionGranularity,
    #[case] response_id: &str,
    #[case] expected: &str,
) {
    let mut metadata = record("{}");
    stamp_cold_storage_object_key(
        &mut metadata,
        Some(start_time()),
        Some(response_id),
        Some(&target(s3_path, partition_granularity)),
    );
    assert_eq!(metadata.cold_storage_object_key.as_deref(), Some(expected));
}

#[test]
fn the_file_name_keeps_six_microsecond_digits() {
    let start = NaiveDate::from_ymd_opt(2026, 1, 2)
        .expect("a valid date")
        .and_hms_nano_opt(3, 4, 5, 6_999)
        .expect("a valid time");
    let mut metadata = record("{}");
    stamp_cold_storage_object_key(
        &mut metadata,
        Some(start),
        Some("chatcmpl-1"),
        Some(&target("", PartitionGranularity::Day)),
    );
    assert_eq!(
        metadata.cold_storage_object_key.as_deref(),
        Some("2026-01-02/time-03-04-05-000006_chatcmpl-1.json")
    );
}

#[rstest]
#[case::missing_start_time(None, Some("chatcmpl-1"))]
#[case::missing_response_id(Some(start_time()), None)]
#[case::missing_both(None, None)]
fn a_missing_part_writes_no_key(
    #[case] start_time: Option<NaiveDateTime>,
    #[case] response_id: Option<&str>,
) {
    let mut metadata = record("{}");
    stamp_cold_storage_object_key(
        &mut metadata,
        start_time,
        response_id,
        Some(&target("logs/", PartitionGranularity::Day)),
    );
    assert_eq!(metadata.cold_storage_object_key, None);
}

#[test]
fn without_a_target_a_copied_key_stays() {
    let mut metadata = record(r#"{"cold_storage_object_key": "copied"}"#);
    stamp_cold_storage_object_key(&mut metadata, Some(start_time()), Some("chatcmpl-1"), None);
    assert_eq!(metadata.cold_storage_object_key.as_deref(), Some("copied"));
}

#[test]
fn without_a_target_and_without_parts_a_copied_key_stays() {
    let mut metadata = record(r#"{"cold_storage_object_key": "copied"}"#);
    stamp_cold_storage_object_key(&mut metadata, None, None, None);
    assert_eq!(metadata.cold_storage_object_key.as_deref(), Some("copied"));
}

#[test]
fn a_generated_key_replaces_a_copied_one() {
    let mut metadata = record(r#"{"cold_storage_object_key": "copied"}"#);
    stamp_cold_storage_object_key(
        &mut metadata,
        Some(start_time()),
        Some("chatcmpl-1"),
        Some(&target("logs/", PartitionGranularity::Day)),
    );
    assert_eq!(
        metadata.cold_storage_object_key.as_deref(),
        Some("logs/2026-01-02/time-03-04-05-123456_chatcmpl-1.json")
    );
}

#[test]
fn the_stamp_leaves_every_other_field_alone() {
    let mut metadata = record(
        r#"{"user_api_key_hash": "hash", "user_api_key_alias": "alias", "team_alias": "team",
            "applied_guardrails": ["guard"], "used_client_oauth_token": true}"#,
    );
    let mut expected = metadata.clone();
    expected.cold_storage_object_key =
        Some("logs/2026-01-02/time-03-04-05-123456_chatcmpl-1.json".to_owned());
    stamp_cold_storage_object_key(
        &mut metadata,
        Some(start_time()),
        Some("chatcmpl-1"),
        Some(&target("logs/", PartitionGranularity::Day)),
    );
    assert_eq!(metadata, expected);
}

#[test]
fn the_default_target_has_no_path_and_day_partitions() {
    let default_target = ColdStorageTarget::default();
    assert_eq!(default_target.s3_path, "");
    assert_eq!(
        default_target.partition_granularity,
        PartitionGranularity::Day
    );
}
