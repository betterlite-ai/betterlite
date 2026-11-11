use std::sync::atomic::Ordering;
use std::time::Duration;

use rstest::rstest;

use super::support::{
    AUDIT_URL, MODEL_URL, Reply, Rig, TOKEN_URL, audit_payload, capture_logs, log_models,
    padded_payload, pending_ids, rig,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

const MAX_PAYLOAD_SIZE_BYTES: usize = 1_000_000;

fn awaiting(rig: &Rig) -> bool {
    rig.logger.model.awaiting_retry.load(Ordering::SeqCst)
}

#[tokio::test]
async fn a_413_splits_the_batch_in_halves() {
    let rig = rig(
        vec![
            Reply::token(),
            Reply::status(413),
            Reply::status(204),
            Reply::status(204),
        ],
        4,
        50_000,
    );
    log_models(&rig, &["e1", "e2", "e3", "e4"]).await;
    assert_eq!(
        rig.transport.batch_ids(),
        [
            vec!["e1", "e2", "e3", "e4"],
            vec!["e1", "e2"],
            vec!["e3", "e4"]
        ]
    );
    assert_eq!(
        rig.transport
            .urls()
            .iter()
            .filter(|url| *url == TOKEN_URL)
            .count(),
        1
    );
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(!awaiting(&rig));
}

#[tokio::test]
async fn a_413_on_a_single_record_drops_it_with_an_error() {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::token(), Reply::status(413)], 1, 50_000);
    log_models(&rig, &["e1"]).await;
    assert_eq!(rig.transport.batch_ids(), [["e1"]]);
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(!awaiting(&rig));
    assert_eq!(
        logs.count_containing("Azure Sentinel API Error - Payload too large for a single record"),
        1
    );
}

#[tokio::test]
async fn a_batch_over_a_million_bytes_splits_before_any_request() {
    let rig = rig(
        vec![Reply::token(), Reply::status(204), Reply::status(204)],
        2,
        50_000,
    );
    rig.logger
        .async_log_model_event(Some(&padded_payload("e1", 600_000)))
        .await;
    rig.logger
        .async_log_model_event(Some(&padded_payload("e2", 600_000)))
        .await;
    assert_eq!(rig.transport.batch_ids(), [["e1"], ["e2"]]);
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
}

fn encoded_len(first_padding: usize) -> usize {
    let pair: Vec<StandardLoggingPayload> =
        vec![padded_payload("e1", first_padding), padded_payload("e2", 0)];
    safe_dumps(&pair, DEFAULT_MAX_RECURSE_DEPTH, None)
        .expect("the pair serializes")
        .len()
}

#[rstest]
#[case::at_the_limit(0, vec![vec!["e1", "e2"]])]
#[case::one_byte_over(1, vec![vec!["e1"], vec!["e2"]])]
#[tokio::test]
async fn the_byte_limit_is_exclusive(#[case] over: usize, #[case] sent: Vec<Vec<&str>>) {
    let target = MAX_PAYLOAD_SIZE_BYTES.saturating_add(over);
    let padding = target.saturating_sub(encoded_len(0));
    assert_eq!(encoded_len(padding), target);
    let rig = rig(
        vec![Reply::token(), Reply::status(204), Reply::status(204)],
        2,
        50_000,
    );
    rig.logger
        .async_log_model_event(Some(&padded_payload("e1", padding)))
        .await;
    rig.logger
        .async_log_model_event(Some(&padded_payload("e2", 0)))
        .await;
    assert_eq!(rig.transport.batch_ids(), sent);
}

#[tokio::test]
async fn a_queue_longer_than_the_batch_size_splits_by_count() {
    let rig = rig(
        vec![
            Reply::token(),
            Reply::status(500),
            Reply::status(204),
            Reply::status(204),
        ],
        2,
        50_000,
    );
    log_models(&rig, &["e1", "e2", "e3"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2", "e3"]);
    rig.logger.flush_queue().await;
    assert_eq!(
        rig.transport.batch_ids(),
        [vec!["e1", "e2"], vec!["e1"], vec!["e2", "e3"]]
    );
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(!awaiting(&rig));
}

#[rstest]
#[case::ok(200, true)]
#[case::no_content(204, true)]
#[case::accepted(202, false)]
#[tokio::test]
async fn only_200_and_204_count_as_delivered(#[case] status: u16, #[case] delivered: bool) {
    let rig = rig(vec![Reply::token(), Reply::status(status)], 1, 50_000);
    log_models(&rig, &["e1"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue).is_empty(), delivered);
    assert_eq!(awaiting(&rig), !delivered);
}

#[rstest]
#[case::bad_request(400, false)]
#[case::unauthorized(401, false)]
#[case::request_timeout(408, true)]
#[case::too_many_requests(429, true)]
#[case::server_error(500, true)]
#[case::bad_gateway(502, true)]
#[tokio::test]
async fn a_rejected_batch_is_kept_only_when_a_retry_can_help(
    #[case] status: u16,
    #[case] kept: bool,
) {
    let rig = rig(vec![Reply::token(), Reply::status(status)], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(
        pending_ids(&rig.logger.model.queue),
        if kept {
            vec!["e1", "e2"]
        } else {
            Vec::<&str>::new()
        }
    );
    assert_eq!(awaiting(&rig), kept);
}

#[tokio::test]
async fn a_failed_right_half_is_requeued_and_the_delivered_left_half_is_not() {
    let rig = rig(
        vec![
            Reply::token(),
            Reply::status(413),
            Reply::status(204),
            Reply::status(500),
        ],
        4,
        50_000,
    );
    log_models(&rig, &["e1", "e2", "e3", "e4"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e3", "e4"]);
    assert!(awaiting(&rig));
}

#[tokio::test]
async fn a_failed_left_half_requeues_the_right_half_unsent() {
    let rig = rig(
        vec![Reply::token(), Reply::status(413), Reply::status(500)],
        4,
        50_000,
    );
    log_models(&rig, &["e1", "e2", "e3", "e4"]).await;
    assert_eq!(
        rig.transport.batch_ids(),
        [vec!["e1", "e2", "e3", "e4"], vec!["e1", "e2"]]
    );
    assert_eq!(
        pending_ids(&rig.logger.model.queue),
        ["e1", "e2", "e3", "e4"]
    );
}

#[tokio::test]
async fn a_network_failure_keeps_the_whole_batch() {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::token(), Reply::Network], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(awaiting(&rig));
    assert_eq!(logs.count_containing("the batch delivery send failed"), 1);
}

#[rstest]
#[case::server_error(500, true)]
#[case::unauthorized(401, false)]
#[case::throttled(429, true)]
#[tokio::test]
async fn a_refused_token_keeps_the_batch_only_when_a_retry_can_help(
    #[case] status: u16,
    #[case] kept: bool,
) {
    let rig = rig(vec![Reply::status(status)], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(rig.transport.urls(), [TOKEN_URL]);
    assert_eq!(
        pending_ids(&rig.logger.model.queue).len(),
        if kept { 2 } else { 0 }
    );
    assert_eq!(awaiting(&rig), kept);
}

#[rstest]
#[case::created(201, "Azure Sentinel Error getting OAuth token")]
#[case::partial(299, "Azure Sentinel Error getting OAuth token")]
#[case::redirect(302, "a status a retry can clear")]
#[case::informational(199, "a status a retry can clear")]
#[tokio::test]
async fn a_token_status_outside_2xx_is_an_http_error_and_any_other_is_a_token_error(
    #[case] status: u16,
    #[case] logged: &str,
) {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::status(status)], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(awaiting(&rig));
    assert_eq!(logs.count_containing(logged), 1);
}

#[tokio::test]
async fn an_unreachable_token_endpoint_keeps_the_batch() {
    let (_guard, logs) = capture_logs();
    let rig = rig(vec![Reply::Network], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(awaiting(&rig));
    assert_eq!(
        logs.count_containing("Azure Sentinel Error getting OAuth token"),
        1
    );
}

#[tokio::test]
async fn a_token_without_an_access_token_keeps_the_batch() {
    let rig = rig(vec![Reply::body(200, "{}")], 2, 50_000);
    log_models(&rig, &["e1", "e2"]).await;
    assert_eq!(pending_ids(&rig.logger.model.queue), ["e1", "e2"]);
    assert!(awaiting(&rig));
}

#[tokio::test]
async fn a_successful_flush_clears_the_retry_flag() {
    let rig = rig(
        vec![Reply::token(), Reply::status(500), Reply::status(204)],
        2,
        50_000,
    );
    log_models(&rig, &["e1", "e2"]).await;
    assert!(awaiting(&rig));
    rig.logger.flush_queue().await;
    assert!(pending_ids(&rig.logger.model.queue).is_empty());
    assert!(!awaiting(&rig));
}

#[tokio::test]
async fn flush_queue_sends_the_model_batch_then_the_audit_batch() {
    let rig = rig(
        vec![Reply::token(), Reply::status(204), Reply::status(204)],
        10,
        50_000,
    );
    rig.logger
        .async_log_audit_log_event(audit_payload("a1"))
        .await;
    log_models(&rig, &["m1"]).await;
    rig.clock.advance(30);
    rig.logger.flush_queue().await;
    assert_eq!(rig.transport.urls(), [TOKEN_URL, MODEL_URL, AUDIT_URL]);
    assert_eq!(rig.transport.batch_ids(), [["m1"], ["a1"]]);
    assert_eq!(
        *rig.logger
            .last_flush_time
            .lock()
            .expect("the flush time is readable"),
        Duration::from_secs(1_030)
    );
}

#[tokio::test]
async fn flush_queue_with_empty_queues_sends_nothing_and_stamps_the_time() {
    let rig = rig(Vec::new(), 10, 50_000);
    rig.clock.advance(7);
    rig.logger.flush_queue().await;
    assert!(rig.transport.calls().is_empty());
    assert_eq!(
        *rig.logger
            .last_flush_time
            .lock()
            .expect("the flush time is readable"),
        Duration::from_secs(1_007)
    );
}
