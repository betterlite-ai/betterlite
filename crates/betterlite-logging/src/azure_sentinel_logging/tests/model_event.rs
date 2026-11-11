use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;

use super::support::{
    MODEL_URL, Reply, TOKEN_URL, capture_logs, log_models, model_payload, pending_ids, rig,
};

#[tokio::test]
async fn success_events_queue_until_the_batch_size_is_reached() {
    let rig = rig(vec![Reply::token(), Reply::status(204)], 3, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(rig.transport.calls().is_empty());

    rig.logger
        .async_log_model_event(Some(&model_payload("e3", false)))
        .await;
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert_eq!(rig.transport.urls(), [TOKEN_URL, MODEL_URL]);
    assert_eq!(rig.transport.batch_ids(), [["e1", "e2", "e3"]]);
}

#[tokio::test]
async fn the_batch_request_carries_the_bearer_token() {
    let rig = rig(vec![Reply::token(), Reply::status(200)], 1, 50_000);
    rig.logger
        .async_log_model_event(Some(&model_payload("e1", false)))
        .await;
    let calls = rig.transport.calls();
    assert_eq!(calls[1].header("authorization"), Some("Bearer token-1"));
    assert_eq!(calls[1].header("content-type"), Some("application/json"));
}

#[tokio::test]
async fn failure_events_queue_and_ship_like_success_events() {
    let rig = rig(vec![Reply::token(), Reply::status(204)], 2, 50_000);
    rig.logger
        .async_log_model_event(Some(&model_payload("ok", false)))
        .await;
    rig.logger
        .async_log_model_event(Some(&model_payload("bad", true)))
        .await;
    let entries = rig.transport.calls()[1].entries();
    let statuses: Vec<Option<&Json>> = entries.iter().map(|entry| entry.get("status")).collect();
    assert_eq!(
        statuses,
        [
            Some(&Json::Text("success".to_owned())),
            Some(&Json::Text("failure".to_owned()))
        ]
    );
}

#[tokio::test]
async fn a_full_queue_waiting_for_a_retry_does_not_send_again() {
    let rig = rig(vec![Reply::token(), Reply::status(500)], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert!(rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);

    rig.logger
        .async_log_model_event(Some(&model_payload("e3", false)))
        .await;
    assert_eq!(rig.transport.calls().len(), 2);
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2", "e3"]);
}

#[tokio::test]
async fn success_event_missing_payload() {
    let (_guard, logs) = capture_logs();
    let rig = rig(Vec::new(), 1, 50_000);
    rig.logger.async_log_model_event(None).await;
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(rig.transport.calls().is_empty());
    assert_eq!(
        logs.count_containing("Azure Sentinel: standard_logging_object not found in kwargs"),
        1
    );
}

#[tokio::test]
async fn a_missing_payload_leaves_the_queue_and_the_threshold_alone() {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::token(), Reply::status(204)], 2, 50_000);
    rig.logger
        .async_log_model_event(Some(&model_payload("e1", false)))
        .await;
    rig.logger.async_log_model_event(None).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1"]);
    assert!(rig.transport.calls().is_empty());
    assert_eq!(logs.count_containing("standard_logging_object"), 1);
}
