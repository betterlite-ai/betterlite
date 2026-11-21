use betterlite_config::sections::openai_realtime::DEFAULT_MAX_MESSAGE_SIZE_BYTES;
use betterlite_openai::realtime::message_cap::{
    MESSAGE_TOO_BIG_CODE, MESSAGE_TOO_BIG_REASON, oversized_message_close,
};

use super::harness::{HarnessOptions, SourceOutcome, harness};
use crate::api::realtime_contracts::transport::RealtimeMessage;

#[tokio::test]
async fn realtime_backend_size_limit() {
    assert_eq!(DEFAULT_MAX_MESSAGE_SIZE_BYTES, 16_777_216);
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

    let mut options = HarnessOptions::default();
    options.max_message_size_bytes = DEFAULT_MAX_MESSAGE_SIZE_BYTES;
    let mut configured = harness(options, Vec::new());
    configured.run().await.unwrap();
    assert_eq!(
        configured.dials()[0].max_size,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );

    let mut options = HarnessOptions::default();
    options.max_message_size_bytes = 0;
    options.source_frames = vec![RealtimeMessage::Text(String::new())];
    let mut empty_frame = harness(options, Vec::new());
    empty_frame.run().await.unwrap();
    match &empty_frame.observation.lock().unwrap().source_outcome {
        SourceOutcome::Text(text) => assert!(text.is_empty()),
        SourceOutcome::Unread | SourceOutcome::Closed { .. } => {
            panic!("an empty frame fits under a zero-byte cap");
        }
    }

    let mut options = HarnessOptions::default();
    options.max_message_size_bytes = 0;
    options.source_frames = vec![RealtimeMessage::Text("a".to_owned())];
    let mut zero_cap = harness(options, Vec::new());
    zero_cap.run().await.unwrap();
    match &zero_cap.observation.lock().unwrap().source_outcome {
        SourceOutcome::Closed {
            code,
            reason,
            message,
        } => {
            assert_eq!(*code, Some(MESSAGE_TOO_BIG_CODE));
            assert_eq!(reason.as_deref(), Some(MESSAGE_TOO_BIG_REASON));
            assert_eq!(
                message.as_str(),
                "the realtime backend message exceeds the 0-byte limit"
            );
        }
        SourceOutcome::Unread | SourceOutcome::Text(_) => {
            panic!("a nonempty frame exceeds a zero-byte cap");
        }
    }

    let mut options = HarnessOptions::default();
    options.max_message_size_bytes = 4;
    options.source_frames = vec![RealtimeMessage::Text("abcd".to_owned())];
    let mut at_cap = harness(options, Vec::new());
    at_cap.run().await.unwrap();
    match &at_cap.observation.lock().unwrap().source_outcome {
        SourceOutcome::Text(text) => assert_eq!(text, "abcd"),
        SourceOutcome::Unread | SourceOutcome::Closed { .. } => {
            panic!("a frame at the cap is accepted");
        }
    }

    let mut options = HarnessOptions::default();
    options.max_message_size_bytes = 4;
    options.source_frames = vec![RealtimeMessage::Binary(vec![1, 2, 3, 4, 5])];
    let mut over_cap = harness(options, Vec::new());
    over_cap.run().await.unwrap();
    match &over_cap.observation.lock().unwrap().source_outcome {
        SourceOutcome::Closed { code, message, .. } => {
            assert_eq!(*code, Some(MESSAGE_TOO_BIG_CODE));
            assert_eq!(
                message.as_str(),
                "the realtime backend message exceeds the 4-byte limit"
            );
        }
        SourceOutcome::Unread | SourceOutcome::Text(_) => {
            panic!("a binary frame over the cap closes the session");
        }
    }
}
