#![cfg(test)]

use super::shared::{Harness, Kind, chat, ok, tokens_request};

#[tokio::test]
async fn apply_to_output_masks_the_stream() {
    let mut harness = Harness::new();
    harness.analyzer.settings.apply_to_output = true;
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("Alice"), chat("!")]))
        .await;
    result.unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains(r#""content":"MASKED""#));
}

#[tokio::test]
async fn apply_to_output_wins_over_output_parse_pii() {
    let mut harness = Harness::new();
    harness.analyzer.settings.apply_to_output = true;
    harness.analyzer.settings.output_parse_pii = true;
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("<PERSON_1>")]))
        .await;
    result.unwrap();
    assert!(sent[0].contains(r#""content":"MASKED""#));
}

#[tokio::test]
async fn a_reader_that_went_away_ends_the_masking_run_without_an_error() {
    let mut harness = Harness::new();
    harness.analyzer.settings.apply_to_output = true;
    harness.reader_gone = true;
    let mut request = tokens_request();
    let (sent, result) = harness
        .run(Kind::Hook, &mut request, ok(vec![chat("Alice")]))
        .await;
    result.unwrap();
    assert!(sent.is_empty());
}
