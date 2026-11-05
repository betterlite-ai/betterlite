#![cfg(test)]

use std::time::Duration;

use super::shared::{
    NO_OVERRIDES, analyzer, find_secret, settings, spawn_mock, text_with_secret_at_33,
};
use crate::presidio_analysis::split::json_escaped_len;
use crate::presidio_analysis::wire::{AnalyzeOutcome, MockRedactedText};

#[tokio::test]
async fn chunking_analyzes_each_chunk_and_merges_the_overlapping_detection() {
    let mock = spawn_mock(find_secret, Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    let AnalyzeOutcome::Items(items) = analyzer(configured)
        .analyze_text(&text_with_secret_at_33(), &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].start, items[0].end), (Some(33), Some(39)));
    let widths: Vec<usize> = mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|request| json_escaped_len(request.text.chars()))
        .collect();
    assert_eq!(widths.len(), 4);
    assert!(widths.iter().all(|&width| width <= 20));
}

#[tokio::test]
async fn chunking_keeps_text_at_exactly_the_budget_in_one_request() {
    let mock = spawn_mock(find_secret, Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    analyzer(configured)
        .analyze_text(&"a".repeat(20), &NO_OVERRIDES)
        .await
        .unwrap();
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn chunking_starts_one_character_over_the_budget() {
    let mock = spawn_mock(find_secret, Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    analyzer(configured)
        .analyze_text(&"a".repeat(21), &NO_OVERRIDES)
        .await
        .unwrap();
    assert_eq!(mock.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn chunking_budgets_the_escaped_width_of_non_ascii_text() {
    let mock = spawn_mock(find_secret, Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    analyzer(configured)
        .analyze_text(&"\u{e9}".repeat(5), &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(mock.seen.lock().unwrap().len() > 1);
}

#[tokio::test]
async fn chunking_sends_a_single_character_in_one_request_even_over_the_budget() {
    let mock = spawn_mock(find_secret, Duration::ZERO).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 4;
    analyzer(configured)
        .analyze_text("\u{1f600}", &NO_OVERRIDES)
        .await
        .unwrap();
    assert_eq!(mock.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn chunking_is_skipped_when_a_mock_redacted_text_is_set() {
    let mock = spawn_mock(find_secret, Duration::ZERO).await;
    let redacted = MockRedactedText {
        text: "<PERSON>".to_owned(),
        items: Vec::new(),
    };
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 4;
    configured.mock_redacted_text = Some(redacted.clone());
    let outcome = analyzer(configured)
        .analyze_text(&text_with_secret_at_33(), &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Redacted(found) if found == redacted));
    assert!(mock.seen.lock().unwrap().is_empty());
}
