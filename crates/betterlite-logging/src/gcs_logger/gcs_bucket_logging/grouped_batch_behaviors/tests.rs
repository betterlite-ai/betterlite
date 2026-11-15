#![cfg(test)]

use betterlite_models::decisions::Json;

use super::super::tests::support::{Answer, body_text, dynamic, item, now, open, upload_name};
use super::send_grouped_batch;

#[tokio::test]
async fn a_group_becomes_one_ndjson_upload() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![
        item("chatcmpl-1", None, Json::Null),
        item("chatcmpl-2", None, Json::Null),
    ];
    let counts = send_grouped_batch(&harness.logger, &items).await;
    assert_eq!(counts, (2, 0));
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url().contains("/b/bucket/o"));
    let batch_name = format!(
        "2024-03-09/batch-{}-00000000.ndjson",
        now().timestamp_millis()
    );
    assert_eq!(
        upload_name(&requests[0]).as_deref(),
        Some(batch_name.as_str())
    );
    let body = body_text(&requests[0]).expect("the batch is sent as text");
    assert_eq!(body.split('\n').count(), 2);
    assert!(body.contains("\"id\":\"chatcmpl-1\""));
    assert!(body.contains("\"id\":\"chatcmpl-2\""));
}

#[tokio::test]
async fn a_failed_upload_counts_every_item() {
    let harness = open(&[], Some("bucket"), Answer::with_status(500));
    let items = vec![
        item("chatcmpl-1", None, Json::Null),
        item("chatcmpl-2", None, Json::Null),
    ];
    let counts = send_grouped_batch(&harness.logger, &items).await;
    assert_eq!(counts, (0, 2));
    assert_eq!(harness.requests().len(), 1);
}

#[tokio::test]
async fn an_empty_group_sends_nothing() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let counts = send_grouped_batch(&harness.logger, &[]).await;
    assert_eq!(counts, (0, 0));
    assert!(harness.requests().is_empty());
}

#[tokio::test]
async fn the_first_item_config_picks_the_bucket() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![
        item("chatcmpl-1", None, dynamic(&[("gcs_bucket_name", "first")])),
        item(
            "chatcmpl-2",
            None,
            dynamic(&[("gcs_bucket_name", "second")]),
        ),
    ];
    let counts = send_grouped_batch(&harness.logger, &items).await;
    assert_eq!(counts, (2, 0));
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url().contains("/b/first/o"));
}

#[tokio::test]
async fn an_invalid_dynamic_config_counts_every_item_as_an_error() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![item("chatcmpl-1", None, Json::Bool(true))];
    let counts = send_grouped_batch(&harness.logger, &items).await;
    assert_eq!(counts, (0, 1));
    assert!(harness.requests().is_empty());
}
