use std::sync::atomic::Ordering;

use super::support::{
    Reply, arrive_audits_during_the_next_send, arrive_models_during_the_next_send, audit_ids,
    capture_logs, log_audits, log_models, model_ids, model_payload, rig,
};

#[tokio::test]
async fn survivors_go_back_ahead_of_the_arrivals() {
    let rig = rig(vec![Reply::token(), Reply::status(500)], 2, 50_000);
    arrive_models_during_the_next_send(&rig, &["e3", "e4"]);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(model_ids(&rig.logger), ["e1", "e2", "e3", "e4"]);
    assert!(rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
}

#[tokio::test]
async fn nothing_is_dropped_while_the_queue_fits() {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::token(), Reply::status(500)], 2, 4);
    arrive_models_during_the_next_send(&rig, &["e3", "e4"]);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(model_ids(&rig.logger), ["e1", "e2", "e3", "e4"]);
    assert_eq!(logs.count_containing("exceeded max_queue_size"), 0);
}

#[tokio::test]
async fn overflow_drops_the_oldest_records_with_a_warning() {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::token(), Reply::status(500)], 2, 3);
    arrive_models_during_the_next_send(&rig, &["e3", "e4"]);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(model_ids(&rig.logger), ["e2", "e3", "e4"]);
    assert_eq!(logs.count_containing("exceeded max_queue_size"), 1);
}

#[tokio::test]
async fn the_audit_queue_overflows_on_its_own() {
    let rig = rig(vec![Reply::token(), Reply::status(500)], 2, 3);
    arrive_audits_during_the_next_send(&rig, &["a3", "a4"]);
    rig.logger
        .async_log_model_event(Some(&model_payload("m1", false)))
        .await;
    log_audits(&rig, &["a1", "a2"]).await;
    assert_eq!(audit_ids(&rig.logger), ["a2", "a3", "a4"]);
    assert_eq!(model_ids(&rig.logger), ["m1"]);
    assert!(rig.logger.audit.awaiting_retry.load(Ordering::SeqCst));
    assert!(!rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
}

#[tokio::test]
async fn appending_past_max_queue_size_drops_the_oldest() {
    let (_guard, logs) = capture_logs();
    let rig = rig(Vec::new(), 10, 2);
    log_models(&rig, &["e1", "e2", "e3"]).await;
    assert_eq!(model_ids(&rig.logger), ["e2", "e3"]);
    assert_eq!(logs.count_containing("exceeded max_queue_size"), 1);
}
