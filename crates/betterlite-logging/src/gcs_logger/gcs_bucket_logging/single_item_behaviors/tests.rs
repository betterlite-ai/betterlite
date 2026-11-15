#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::super::tests::support::{
    Answer, body_of, capture_logs, item, metadata, open, upload_name,
};
use super::send_single_log_item;

const FIRST_ID: &str = "00000000000000000000000000000000";

#[tokio::test]
async fn a_success_item_is_uploaded_under_the_response_id() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    send_single_log_item(&harness.logger, &item("chatcmpl-1", None, Json::Null)).await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        upload_name(&requests[0]).as_deref(),
        Some("2024-03-09/chatcmpl-1")
    );
    let body = body_of(&requests[0]).expect("the body is JSON");
    let Json::Object(fields) = body else {
        panic!("the body is a JSON object");
    };
    assert_eq!(fields["id"], Json::Text("chatcmpl-1".to_owned()));
    assert_eq!(fields["status"], Json::Text("success".to_owned()));
}

#[tokio::test]
async fn a_failure_item_is_uploaded_under_a_failure_uuid() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    send_single_log_item(
        &harness.logger,
        &item("chatcmpl-1", Some("the upstream call failed"), Json::Null),
    )
    .await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    let expected = format!("2024-03-09/failure-{FIRST_ID}");
    assert_eq!(
        upload_name(&requests[0]).as_deref(),
        Some(expected.as_str())
    );
}

#[tokio::test]
async fn a_custom_log_id_names_the_object() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let mut queued = item("chatcmpl-1", None, Json::Null);
    queued.metadata = Arc::new(metadata("gcs_log_id", Json::Text("my-run".to_owned())));
    send_single_log_item(&harness.logger, &queued).await;
    let expected = format!("2024-03-09/custom-{FIRST_ID}-my-run");
    assert_eq!(
        upload_name(&harness.requests()[0]).as_deref(),
        Some(expected.as_str())
    );
}

#[tokio::test]
async fn an_upload_failure_is_logged_and_not_propagated() {
    let harness = open(&[], Some("bucket"), Answer::with_status(500));
    let (logs, _guard) = capture_logs();
    send_single_log_item(&harness.logger, &item("chatcmpl-1", None, Json::Null)).await;
    assert_eq!(harness.requests().len(), 1);
    assert!(
        logs.text()
            .contains("GCS Bucket error logging individual payload to GCS bucket")
    );
}

#[tokio::test]
async fn a_non_string_custom_log_id_skips_the_upload() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let mut queued = item("chatcmpl-1", None, Json::Null);
    queued.metadata = Arc::new(metadata("gcs_log_id", Json::Bool(true)));
    send_single_log_item(&harness.logger, &queued).await;
    assert!(harness.requests().is_empty());
}
