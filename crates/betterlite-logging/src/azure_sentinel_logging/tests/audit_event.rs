use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;

use super::support::{AUDIT_URL, Reply, TOKEN_URL, audit_payload, model_payload, pending_ids, rig};

#[tokio::test]
async fn audit_events_queue_apart_from_model_events() {
    let rig = rig(Vec::new(), 5, 50_000);
    rig.logger
        .async_log_audit_log_event(audit_payload("a1"))
        .await;
    rig.logger
        .async_log_model_event(Some(&model_payload("m1", false)))
        .await;
    assert_eq!(pending_ids(&rig.logger.audit.queue), ["a1"]);
    assert_eq!(pending_ids(&rig.logger.model.queue), ["m1"]);
}

#[tokio::test]
async fn audit_events_send_at_the_batch_size_to_the_audit_stream() {
    let rig = rig(vec![Reply::token(), Reply::status(204)], 2, 50_000);
    rig.logger
        .async_log_audit_log_event(audit_payload("a1"))
        .await;
    assert!(rig.transport.calls().is_empty());
    rig.logger
        .async_log_audit_log_event(audit_payload("a2"))
        .await;
    assert!(pending_ids(&rig.logger.audit.queue).is_empty());
    assert_eq!(rig.transport.urls(), [TOKEN_URL, AUDIT_URL]);
    assert_eq!(rig.transport.batch_ids(), [["a1", "a2"]]);
}

#[tokio::test]
async fn audit_batches_serialize_the_audit_fields() {
    let rig = rig(vec![Reply::token(), Reply::status(200)], 1, 50_000);
    rig.logger
        .async_log_audit_log_event(audit_payload("a1"))
        .await;
    let entries = rig.transport.calls()[1].entries();
    let fields = &entries[0];
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "action",
            "before_value",
            "changed_by",
            "changed_by_api_key",
            "id",
            "object_id",
            "table_name",
            "updated_at",
            "updated_values"
        ]
    );
    assert_eq!(fields.get("before_value"), Some(&Json::Null));
}

#[tokio::test]
async fn audit_events_wait_for_a_retry_independently_of_model_events() {
    let rig = rig(vec![Reply::token(), Reply::status(500)], 1, 50_000);
    rig.logger
        .async_log_audit_log_event(audit_payload("a1"))
        .await;
    assert!(rig.logger.audit.awaiting_retry.load(Ordering::SeqCst));
    assert!(!rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
    rig.logger
        .async_log_audit_log_event(audit_payload("a2"))
        .await;
    assert_eq!(rig.transport.calls().len(), 2);
    assert_eq!(pending_ids(&rig.logger.audit.queue), ["a1", "a2"]);
}
