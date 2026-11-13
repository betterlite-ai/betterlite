#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use super::shared::{NO_OVERRIDES, analyzer, json_reply, settings, spawn_mock};
use crate::presidio_analysis::wire::{AnalyzeOutcome, MockRedactedText};

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::newlines_and_tabs("\n\t \r\n")]
#[tokio::test]
async fn empty_text_sends_no_request_and_returns_no_items(#[case] text: &str) {
    let mock = spawn_mock(
        |_| json_reply(r#"[{"entity_type":"PERSON","start":0,"end":1,"score":1.0}]"#),
        Duration::ZERO,
    )
    .await;
    let outcome = analyzer(settings(&mock.base))
        .analyze_text(text, &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Items(found) if found.is_empty()));
    assert!(mock.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn empty_text_beats_the_mock_redacted_text() {
    let mock = spawn_mock(|_| json_reply("[]"), Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.mock_redacted_text = Some(MockRedactedText {
        text: "<REDACTED>".to_owned(),
        items: Vec::new(),
    });
    let outcome = analyzer(configured)
        .analyze_text("  ", &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Items(found) if found.is_empty()));
}

#[tokio::test]
async fn empty_text_chunks_of_blank_text_send_no_request() {
    let mock = spawn_mock(|_| json_reply("[]"), Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 4;
    let outcome = analyzer(configured)
        .analyze_text(&" ".repeat(40), &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Items(found) if found.is_empty()));
    assert!(mock.seen.lock().unwrap().is_empty());
}
