#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::super::Data;
use super::super::error_handling::{FieldError, HookError};
use super::{
    bucket_key, record_text_scan, recorded_response, recorded_status, request_start_time,
    scan_files,
};
use crate::guardrail_records::record::GuardrailStatus;
use crate::model_armor::hooks::tests::support::{
    BLOCKED, CLEAN, NAME, armor, attachment_data, blocked, bucket, data, file_settings, logged,
    messages, prompt_data, text, value,
};
use crate::model_armor::transport::error::ArmorTokenError;

#[test]
fn metadata_bucket_prefers_litellm_metadata() {
    let mut request = data(r#"{"metadata":{},"litellm_metadata":{}}"#);
    assert_eq!(bucket_key(&request), "litellm_metadata");
    assert!(request.remove("litellm_metadata").is_some());
    assert_eq!(bucket_key(&request), "metadata");
    assert_eq!(bucket_key(&prompt_data()), "metadata");
}

#[test]
fn metadata_response_defaults_to_an_empty_object() {
    let request = prompt_data();
    assert_eq!(
        recorded_response(&request),
        GuardrailValue::Object(BTreeMap::new())
    );
    let with = data(r#"{"metadata":{"_model_armor_response":{"a":1}}}"#);
    assert_eq!(recorded_response(&with), value(r#"{"a":1}"#));
    let litellm = data(r#"{"litellm_metadata":{"_model_armor_response":[]}}"#);
    assert_eq!(recorded_response(&litellm), value("[]"));
}

#[test]
fn metadata_reads_a_bucket_that_is_not_an_object_as_missing() {
    let request = data(r#"{"metadata":"text"}"#);
    assert_eq!(
        recorded_response(&request),
        GuardrailValue::Object(BTreeMap::new())
    );
    assert_eq!(recorded_status(&request), GuardrailStatus::Success);
}

#[rstest]
#[case::missing(None, GuardrailStatus::Success)]
#[case::blocked(Some("blocked"), GuardrailStatus::GuardrailIntervened)]
#[case::success(Some("success"), GuardrailStatus::Success)]
#[case::unknown(Some("weird"), GuardrailStatus::Success)]
fn metadata_maps_the_stored_status(
    #[case] status: Option<&str>,
    #[case] expected: GuardrailStatus,
) {
    let mut request = prompt_data();
    if let Some(status) = status {
        request.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(BTreeMap::from([(
                "_model_armor_status".to_owned(),
                GuardrailValue::Text(status.to_owned()),
            )])),
        );
    }
    assert_eq!(recorded_status(&request), expected);
}

#[tokio::test]
async fn metadata_warns_about_an_unknown_status() {
    let request = data(r#"{"metadata":{"_model_armor_status":"weird"}}"#);
    let mut status = GuardrailStatus::NotRun;
    let log = logged(async {
        status = recorded_status(&request);
    })
    .await;
    assert_eq!(status, GuardrailStatus::Success);
    assert!(log.contains("unknown status in the metadata bucket"));
}

#[test]
fn metadata_scan_appends_each_response_and_sets_the_status() {
    let mut request = prompt_data();
    record_text_scan(&mut request, value(CLEAN), false);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_response"),
        Some(&value(CLEAN))
    );
    record_text_scan(&mut request, value(BLOCKED), true);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("blocked"))
    );
    match bucket(&request, "metadata").get("_model_armor_response") {
        Some(GuardrailValue::Array(items)) => assert_eq!(items.len(), 2),
        other => panic!("expected two responses, got {other:?}"),
    }
}

#[test]
fn metadata_a_blocked_status_stays_sticky() {
    let mut request = data(r#"{"metadata":{"_model_armor_status":"blocked","caller":"kept"}}"#);
    record_text_scan(&mut request, value(CLEAN), false);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("blocked"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("caller"),
        Some(&text("kept"))
    );
}

#[test]
fn metadata_scan_writes_the_litellm_metadata_bucket_when_present() {
    let mut request = data(r#"{"litellm_metadata":{},"metadata":{}}"#);
    record_text_scan(&mut request, value(CLEAN), false);
    assert_eq!(
        bucket(&request, "litellm_metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
    assert!(bucket(&request, "metadata").is_empty());
}

#[test]
fn metadata_scan_replaces_a_bucket_that_is_not_an_object() {
    let mut request = data(r#"{"metadata":"text"}"#);
    record_text_scan(&mut request, value(CLEAN), false);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
}

#[rstest]
#[case::absent("{}", Ok(None))]
#[case::null(r#"{"start_time":null}"#, Ok(None))]
#[case::fraction(r#"{"start_time":1.5}"#, Ok(Some(1.5)))]
#[case::integer(r#"{"start_time":7}"#, Ok(Some(7.0)))]
#[case::text(
    r#"{"start_time":"soon"}"#,
    Err(FieldError { field: "start_time", expected: "a number" })
)]
#[case::boolean(
    r#"{"start_time":true}"#,
    Err(FieldError { field: "start_time", expected: "a number" })
)]
fn metadata_reads_the_request_start_time(
    #[case] source: &str,
    #[case] expected: Result<Option<f64>, FieldError>,
) {
    let request = data(source);
    assert_eq!(request_start_time(&request), expected);
}

#[test]
fn metadata_rejects_a_number_marker_with_other_keys() {
    let request = data(r#"{"start_time":{"$serde_json::private::Number":"1.5","extra":true}}"#);
    assert!(request_start_time(&request).is_err());
}

#[test]
fn metadata_reads_a_start_time_that_came_from_a_stored_record() {
    let mut request = Data::new();
    request.insert("start_time".to_owned(), GuardrailValue::Decimal(2.5));
    assert_eq!(request_start_time(&request).expect("start"), Some(2.5));
    let marker = data(r#"{"start_time":3.25}"#);
    assert_eq!(request_start_time(&marker).expect("start"), Some(3.25));
}

#[tokio::test]
async fn metadata_file_scan_does_not_touch_a_request_without_files() {
    let fixture = armor(Vec::new()).await;
    let mut request = prompt_data();
    let before = request.clone();
    let plain = messages(r#"[{"role":"user","content":"hello"}]"#);
    scan_files(
        &mut request,
        &plain,
        &fixture.transport,
        &file_settings(true),
    )
    .await
    .expect("scan");
    assert_eq!(request, before);
    assert_eq!(fixture.calls(), 0);
}

#[tokio::test]
async fn metadata_file_scan_writes_the_bucket() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let mut request = attachment_data();
    let scanned = messages(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]"#,
    );
    scan_files(
        &mut request,
        &scanned,
        &fixture.transport,
        &file_settings(true),
    )
    .await
    .expect("scan");
    assert_eq!(fixture.calls(), 1);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
    assert!(bucket(&request, "metadata").contains_key("_model_armor_response"));
}

#[tokio::test]
async fn metadata_file_scan_keeps_the_existing_bucket_keys() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let mut request = data(
        r#"{"metadata":{"caller":"kept"},"messages":[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]}"#,
    );
    let scanned = messages(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]"#,
    );
    scan_files(
        &mut request,
        &scanned,
        &fixture.transport,
        &file_settings(true),
    )
    .await
    .expect("scan");
    assert_eq!(
        bucket(&request, "metadata").get("caller"),
        Some(&text("kept"))
    );
    assert!(bucket(&request, "metadata").contains_key("_model_armor_status"));
}

#[tokio::test]
async fn metadata_file_scan_writes_the_litellm_metadata_bucket_when_present() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let mut request = data(
        r#"{"litellm_metadata":{},"messages":[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]}"#,
    );
    let scanned = messages(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]"#,
    );
    scan_files(
        &mut request,
        &scanned,
        &fixture.transport,
        &file_settings(true),
    )
    .await
    .expect("scan");
    assert!(request.get("metadata").is_none());
    assert!(bucket(&request, "litellm_metadata").contains_key("_model_armor_status"));
}

#[tokio::test]
async fn metadata_file_scan_surfaces_a_block() {
    let fixture = armor(vec![(StatusCode::OK, BLOCKED.to_owned())]).await;
    let mut request = attachment_data();
    let scanned = messages(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]"#,
    );
    let error = scan_files(
        &mut request,
        &scanned,
        &fixture.transport,
        &file_settings(true),
    )
    .await
    .expect_err("blocked");
    let (status, detail) = blocked(error);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked by Model Armor"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("blocked"))
    );
}

#[tokio::test]
async fn metadata_file_scan_maps_an_api_error() {
    let fixture = armor(vec![(
        StatusCode::BAD_REQUEST,
        r#"{"error":"bad"}"#.to_owned(),
    )])
    .await;
    let mut request = attachment_data();
    let scanned = messages(
        r#"[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]"#,
    );
    let error = scan_files(
        &mut request,
        &scanned,
        &fixture.transport,
        &file_settings(true),
    )
    .await
    .expect_err("api error");
    assert!(matches!(error, HookError::<ArmorTokenError>::Api(_)));
}
