use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::timeout;

use super::support::{Reply, Rig, audit_payload, log_models, model_payload, pending_ids, rig};

const GIVE_UP: Duration = Duration::from_secs(1);

async fn cancel_flush(rig: &Rig) {
    timeout(GIVE_UP, rig.logger.flush_queue())
        .await
        .expect_err("the flush never finishes");
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_flush_keeps_the_records_and_marks_the_queue_for_retry() {
    let rig = rig(vec![Reply::token(), Reply::Pending], 3, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    cancel_flush(&rig).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn delivered_halves_are_not_resubmitted_after_a_cancel() {
    let rig = rig(
        vec![
            Reply::token(),
            Reply::status(413),
            Reply::status(204),
            Reply::Pending,
            Reply::status(204),
        ],
        4,
        50_000,
    );
    log_models(&rig, &["e1", "e2", "e3"]).await;
    timeout(
        GIVE_UP,
        rig.logger
            .async_log_model_event(Some(&model_payload("e4", false))),
    )
    .await
    .expect_err("the send never finishes");
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e3", "e4"]);
    assert!(rig.logger.model.awaiting_retry.load(Ordering::SeqCst));

    rig.logger.flush_queue().await;
    assert_eq!(
        rig.transport.batch_ids(),
        [
            vec!["e1", "e2", "e3", "e4"],
            vec!["e1", "e2"],
            vec!["e3", "e4"],
            vec!["e3", "e4"]
        ]
    );
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(!rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn a_cancel_in_the_left_half_keeps_both_halves() {
    let rig = rig(
        vec![Reply::token(), Reply::status(413), Reply::Pending],
        4,
        50_000,
    );
    log_models(&rig, &["e1", "e2", "e3"]).await;
    timeout(
        GIVE_UP,
        rig.logger
            .async_log_model_event(Some(&model_payload("e4", false))),
    )
    .await
    .expect_err("the send never finishes");
    assert_eq!(
        pending_ids(&rig.logger.model.queue),
        ["e1", "e2", "e3", "e4"]
    );
    assert!(rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn a_cancel_while_fetching_the_token_keeps_the_batch() {
    let rig = rig(vec![Reply::Pending], 2, 50_000);
    log_models(&rig, &["e1"]).await;
    timeout(
        GIVE_UP,
        rig.logger
            .async_log_model_event(Some(&model_payload("e2", false))),
    )
    .await
    .expect_err("the token request never finishes");
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn a_cancel_in_the_audit_send_keeps_the_audit_batch_and_the_flush_time() {
    let rig = rig(
        vec![Reply::token(), Reply::status(204), Reply::Pending],
        10,
        50_000,
    );
    log_models(&rig, &["m1"]).await;
    rig.logger
        .async_log_audit_log_event(audit_payload("a1"))
        .await;
    rig.clock.advance(30);
    cancel_flush(&rig).await;
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(!rig.logger.model.awaiting_retry.load(Ordering::SeqCst));
    assert_eq!(pending_ids(&rig.logger.audit.queue), ["a1"]);
    assert!(rig.logger.audit.awaiting_retry.load(Ordering::SeqCst));
    assert_eq!(
        *rig.logger
            .last_flush_time
            .lock()
            .expect("the flush time is readable"),
        Duration::from_secs(1_000)
    );
}
