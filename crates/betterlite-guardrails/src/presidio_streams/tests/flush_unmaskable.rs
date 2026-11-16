#![cfg(test)]

use serde_json::value::RawValue;
use tokio::sync::mpsc;

use super::shared::{FakeAssembly, capture_logs, chat, chat_chunk, describe, logged_text};
use crate::presidio_streams::error::PresidioStreamError;
use crate::presidio_streams::mask::flush_unmaskable_buffer;

fn chunks() -> Vec<Box<RawValue>> {
    vec![chat_chunk("a"), chat_chunk("b")]
}

#[tokio::test]
async fn warns_and_yields_the_buffered_chunks_in_order() {
    let (captured, _guard) = capture_logs();
    let (out, mut sent) = mpsc::channel(4);
    flush_unmaskable_buffer::<FakeAssembly>(&chunks(), &out)
        .await
        .unwrap();
    drop(out);
    let mut flushed = Vec::new();
    while let Some(event) = sent.recv().await {
        flushed.push(describe(&event));
    }
    assert_eq!(flushed, [describe(&chat("a")), describe(&chat("b"))]);
    let log = logged_text(&captured);
    assert!(log.contains("WARN"));
    assert!(log.contains("mixed stream detected"));
    assert!(log.contains("Flushing 2 buffered chunks without PII masking"));
}

#[tokio::test]
async fn an_empty_buffer_logs_nothing_and_yields_nothing() {
    let (captured, _guard) = capture_logs();
    let (out, mut sent) = mpsc::channel(1);
    flush_unmaskable_buffer::<FakeAssembly>(&[], &out)
        .await
        .unwrap();
    drop(out);
    assert!(sent.recv().await.is_none());
    assert!(logged_text(&captured).is_empty());
}

#[tokio::test]
async fn a_reader_that_went_away_ends_the_flush() {
    let (out, mut sent) = mpsc::channel(1);
    sent.close();
    let error = flush_unmaskable_buffer::<FakeAssembly>(&chunks(), &out)
        .await
        .unwrap_err();
    assert!(matches!(error, PresidioStreamError::ConsumerClosed));
}
