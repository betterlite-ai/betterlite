use betterlite_config::sections::openai_realtime::DEFAULT_MAX_MESSAGE_SIZE_BYTES;
use betterlite_openai::realtime::message_cap::{
    MESSAGE_TOO_BIG_CODE, MESSAGE_TOO_BIG_REASON, oversized_message_close,
};

use super::harness::{HarnessOptions, SourceOutcome, harness};
use crate::api::realtime_contracts::transport::RealtimeMessage;

#[tokio::test]
async fn realtime_backend_size_limit() {
    default_cap_refuses_an_oversized_message();
    passes_the_cap_to_the_dialer().await;
    accepts_an_empty_frame_under_a_zero_cap().await;
    closes_a_nonempty_frame_under_a_zero_cap().await;
    accepts_a_frame_at_the_cap().await;
    closes_a_binary_frame_over_the_cap().await;
}

fn default_cap_refuses_an_oversized_message() {
    assert_eq!(DEFAULT_MAX_MESSAGE_SIZE_BYTES, 0x0100_0000);
    assert!(
        oversized_message_close(
            DEFAULT_MAX_MESSAGE_SIZE_BYTES,
            DEFAULT_MAX_MESSAGE_SIZE_BYTES
        )
        .is_none()
    );
    let over = oversized_message_close(
        DEFAULT_MAX_MESSAGE_SIZE_BYTES + 1,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES,
    )
    .unwrap();
    assert_eq!(over.code, MESSAGE_TOO_BIG_CODE);
    assert_eq!(over.reason, MESSAGE_TOO_BIG_REASON);
}

async fn passes_the_cap_to_the_dialer() {
    let options =
        HarnessOptions::default().with_max_message_size_bytes(DEFAULT_MAX_MESSAGE_SIZE_BYTES);
    let mut configured = harness(options, Vec::new());
    configured.run().await.unwrap();
    assert_eq!(
        configured.dials()[0].max_size,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );
}

async fn outcome_with_cap(max: usize, frames: Vec<RealtimeMessage>) -> SourceOutcome {
    let options = HarnessOptions::default()
        .with_max_message_size_bytes(max)
        .with_source_frames(frames);
    let mut configured = harness(options, Vec::new());
    configured.run().await.unwrap();
    configured.source_outcome()
}

fn assert_closed(outcome: SourceOutcome, expected_message: &str) {
    let SourceOutcome::Closed {
        code,
        reason,
        message,
    } = outcome
    else {
        panic!("a frame over the cap closes the session");
    };
    assert_eq!(code, Some(MESSAGE_TOO_BIG_CODE));
    assert_eq!(reason.as_deref(), Some(MESSAGE_TOO_BIG_REASON));
    assert_eq!(message, expected_message);
}

async fn accepts_an_empty_frame_under_a_zero_cap() {
    let outcome = outcome_with_cap(0, vec![RealtimeMessage::Text(String::new())]).await;
    assert!(matches!(outcome, SourceOutcome::Text(text) if text.is_empty()));
}

async fn closes_a_nonempty_frame_under_a_zero_cap() {
    let outcome = outcome_with_cap(0, vec![RealtimeMessage::Text("a".to_owned())]).await;
    assert_closed(
        outcome,
        "the realtime backend message exceeds the 0-byte limit",
    );
}

async fn accepts_a_frame_at_the_cap() {
    let outcome = outcome_with_cap(4, vec![RealtimeMessage::Text("abcd".to_owned())]).await;
    assert!(matches!(outcome, SourceOutcome::Text(text) if text == "abcd"));
}

async fn closes_a_binary_frame_over_the_cap() {
    let outcome = outcome_with_cap(4, vec![RealtimeMessage::Binary(vec![1, 2, 3, 4, 5])]).await;
    assert_closed(
        outcome,
        "the realtime backend message exceeds the 4-byte limit",
    );
}
