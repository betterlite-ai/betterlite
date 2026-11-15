#![cfg(test)]

use betterlite_models::decisions::Json;

use super::super::tests::support::{Answer, item, open, upload_name};
use super::send_individual_logs;

const FIRST_ID: &str = "00000000000000000000000000000000";

#[tokio::test]
async fn each_item_becomes_its_own_upload() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![
        item("chatcmpl-1", None, Json::Null),
        item("chatcmpl-2", Some("the upstream call failed"), Json::Null),
    ];
    send_individual_logs(&harness.logger, &items).await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 2);
    let names: Vec<String> = requests.iter().filter_map(upload_name).collect();
    assert_eq!(
        names,
        vec![
            "2024-03-09/chatcmpl-1".to_owned(),
            format!("2024-03-09/failure-{FIRST_ID}")
        ]
    );
}

#[tokio::test]
async fn an_upload_failure_does_not_stop_the_loop() {
    let harness = open(&[], Some("bucket"), Answer::with_status(500));
    let items = vec![
        item("chatcmpl-1", None, Json::Null),
        item("chatcmpl-2", None, Json::Null),
    ];
    send_individual_logs(&harness.logger, &items).await;
    assert_eq!(harness.requests().len(), 2);
}

#[tokio::test]
async fn an_invalid_config_does_not_stop_the_loop() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![
        item("chatcmpl-1", None, Json::Bool(true)),
        item("chatcmpl-2", None, Json::Null),
    ];
    send_individual_logs(&harness.logger, &items).await;
    let requests = harness.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        upload_name(&requests[0]).as_deref(),
        Some("2024-03-09/chatcmpl-2")
    );
}

#[tokio::test]
async fn an_empty_batch_sends_nothing() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    send_individual_logs(&harness.logger, &[]).await;
    assert!(harness.requests().is_empty());
}
