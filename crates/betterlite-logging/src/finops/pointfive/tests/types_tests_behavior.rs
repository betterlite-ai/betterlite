#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret;

use super::types::{
    DEFAULT_API_URL, PointFiveInitParams, PointFiveUploadFailure, PointFiveUploadTarget,
    RETRYABLE_UPLOAD_STATUS_CODES,
};

fn parse(source: &str) -> PointFiveInitParams {
    serde_json::from_str(source).expect("the params parse")
}

#[test]
fn defaults_match_litellm() {
    let params = PointFiveInitParams::default();
    assert!(params.api_key.is_none());
    assert!(params.api_url.is_none());
    assert_eq!(params.batch_size, 1_000);
    assert_eq!(params.flush_interval, 300);
    assert_eq!(params.max_batch_bytes, 8 * 1024 * 1024);
    assert_eq!(params.max_upload_retries, 3);
    assert!(!params.turn_off_message_logging);
}

#[test]
fn an_empty_map_takes_every_default() {
    let params = parse("{}");
    assert!(params.api_key.is_none());
    assert!(params.api_url.is_none());
    assert_eq!(params.batch_size, 1_000);
    assert_eq!(params.flush_interval, 300);
    assert_eq!(params.max_batch_bytes, 8 * 1024 * 1024);
    assert_eq!(params.max_upload_retries, 3);
    assert!(!params.turn_off_message_logging);
}

#[test]
fn configured_values_are_read() {
    let params = parse(
        r#"{
            "api_key": "sk-test",
            "api_url": "https://pointfive.test/ingestion",
            "batch_size": 25,
            "flush_interval": 60,
            "max_batch_bytes": 4096,
            "max_upload_retries": 7,
            "turn_off_message_logging": true
        }"#,
    );
    assert_eq!(
        params.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-test")
    );
    assert_eq!(
        params.api_url.as_deref(),
        Some("https://pointfive.test/ingestion")
    );
    assert_eq!(params.batch_size, 25);
    assert_eq!(params.flush_interval, 60);
    assert_eq!(params.max_batch_bytes, 4_096);
    assert_eq!(params.max_upload_retries, 7);
    assert!(params.turn_off_message_logging);
}

#[test]
fn unknown_keys_are_ignored() {
    let params = parse(r#"{"batch_size": 5, "some_other_logger_setting": {"nested": true}}"#);
    assert_eq!(params.batch_size, 5);
    assert_eq!(params.flush_interval, 300);
}

#[test]
fn nulls_take_the_defaults() {
    let params = parse(
        r#"{
            "api_key": null,
            "api_url": null,
            "batch_size": null,
            "flush_interval": null,
            "max_batch_bytes": null,
            "max_upload_retries": null,
            "turn_off_message_logging": null
        }"#,
    );
    assert!(params.api_key.is_none());
    assert!(params.api_url.is_none());
    assert_eq!(params.batch_size, 1_000);
    assert_eq!(params.flush_interval, 300);
    assert_eq!(params.max_batch_bytes, 8 * 1024 * 1024);
    assert_eq!(params.max_upload_retries, 3);
    assert!(!params.turn_off_message_logging);
}

#[rstest]
#[case::batch_size(r#"{"batch_size": 0}"#, "batch_size")]
#[case::flush_interval(r#"{"flush_interval": 0}"#, "flush_interval")]
#[case::max_batch_bytes(r#"{"max_batch_bytes": 0}"#, "max_batch_bytes")]
#[case::max_upload_retries(r#"{"max_upload_retries": 0}"#, "max_upload_retries")]
fn zero_is_rejected(#[case] source: &str, #[case] field: &str) {
    let error = serde_json::from_str::<PointFiveInitParams>(source).expect_err("zero is refused");
    assert!(error.to_string().contains(field), "{error}");
}

#[rstest]
#[case::batch_size(r#"{"batch_size": 1}"#, "batch_size")]
#[case::flush_interval(r#"{"flush_interval": 1}"#, "flush_interval")]
#[case::max_batch_bytes(r#"{"max_batch_bytes": 1}"#, "max_batch_bytes")]
#[case::max_upload_retries(r#"{"max_upload_retries": 1}"#, "max_upload_retries")]
fn one_is_accepted(#[case] source: &str, #[case] field: &str) {
    let params = parse(source);
    let value = match field {
        "batch_size" => params.batch_size,
        "flush_interval" => usize::try_from(params.flush_interval).expect("fits"),
        "max_batch_bytes" => params.max_batch_bytes,
        "max_upload_retries" => params.max_upload_retries,
        other => panic!("unexpected field {other}"),
    };
    assert_eq!(value, 1);
}

#[test]
fn a_label_value_is_not_a_number() {
    let error =
        serde_json::from_str::<PointFiveInitParams>(r#"{"batch_size": "5"}"#).expect_err("text");
    assert!(error.to_string().contains("invalid type"), "{error}");
}

#[test]
fn a_negative_count_is_rejected() {
    let error = serde_json::from_str::<PointFiveInitParams>(r#"{"max_upload_retries": -1}"#)
        .expect_err("-1");
    assert!(error.to_string().contains("invalid value"), "{error}");
}

#[test]
fn upload_records_carry_their_fields() {
    let target = PointFiveUploadTarget {
        upload_url: "https://uploads.example/object".to_owned(),
        object_key: "objects/1".to_owned(),
    };
    assert_eq!(target.upload_url, "https://uploads.example/object");
    assert_eq!(target.object_key, "objects/1");
    let failure = PointFiveUploadFailure {
        detail: "pointfive api returned 500".to_owned(),
        retryable: true,
    };
    assert_eq!(failure.detail, "pointfive api returned 500");
    assert!(failure.retryable);
}

#[test]
fn retryable_status_codes_match_litellm() {
    assert_eq!(RETRYABLE_UPLOAD_STATUS_CODES, [429, 500, 502, 503, 504]);
}

#[test]
fn default_api_url_matches_litellm() {
    assert_eq!(DEFAULT_API_URL, "https://api.pointfive.co/api/v1/ingestion");
}
