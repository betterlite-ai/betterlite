#![cfg(test)]

use std::sync::{Arc, PoisonError};

use betterlite_openai::passthrough::transformation::PassthroughStreamCollector;
use betterlite_openai::passthrough::transformation::types::{
    MAX_COLLECTED_BYTES, PassthroughTarget,
};
use tracing::Level;

use super::{OwnedProviderSpend, provider_spend};
use crate::passthrough_stream::tests::{
    ENDPOINT, FakeConfig, MODEL, NoopPassthroughLogging, PROVIDER, capture, collection, fake_config,
};

fn owned_spend(config: FakeConfig) -> OwnedProviderSpend<FakeConfig> {
    OwnedProviderSpend {
        config,
        model: MODEL.to_owned(),
        custom_llm_provider: PROVIDER.to_owned(),
        endpoint: ENDPOINT.to_owned(),
        chunks: Vec::new(),
        collected_bytes: 0,
        disabled: false,
    }
}

#[test]
fn provider_spend_replays_chunks_into_the_provider_collector() {
    let config = fake_config();
    let collected = Arc::clone(&config.collected);
    let mut spend = provider_spend(
        config,
        PassthroughTarget {
            model: MODEL,
            custom_llm_provider: PROVIDER,
            endpoint: ENDPOINT,
        },
    );
    spend.add(b"data: one\n\n");
    spend.add(b"data: two\n\n");
    let (collector, chunk_count) = spend.take_flush().expect("two chunks flush");
    assert_eq!(chunk_count, 2);
    let mut logging = NoopPassthroughLogging;
    let logged = collector
        .build_logged_response(&mut logging)
        .expect("the provider collector builds")
        .expect("the provider collector logs");
    assert_eq!(logged, "data: one;data: two");
    let records = collected
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model, MODEL);
    assert_eq!(records[0].custom_llm_provider, PROVIDER);
    assert_eq!(records[0].endpoint, ENDPOINT);
    assert_eq!(
        records[0].lines,
        vec!["data: one".to_owned(), "data: two".to_owned()]
    );
}

#[test]
fn add_absorbs_a_collector_failure_and_drops_later_chunks() {
    let (mut spend, state) = collection();
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    let (captured, _guard) = capture();
    spend.add(b"one");
    spend.add(b"two");
    {
        let guarded = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(guarded.calls, 1);
        assert!(guarded.chunks.is_empty());
    }
    assert!(spend.take_flush().is_none());
    let errors: Vec<_> = captured
        .events()
        .into_iter()
        .filter(|event| event.level == Level::ERROR)
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .message
            .contains("Passthrough spend-tracking collector failed; spend dropped for this stream")
    );
    assert!(errors[0].fields.iter().any(|field| field == "chunks=1"));
    assert!(
        errors[0]
            .fields
            .iter()
            .any(|field| field.contains("collector add 1 failed"))
    );
}

#[test]
fn should_flush_needs_chunks_and_a_successful_collector() {
    let (mut empty, _state) = collection();
    assert!(empty.take_flush().is_none());
    let (mut kept, _state) = collection();
    kept.add(b"a");
    assert!(kept.take_flush().is_some());
    let (mut failed, state) = collection();
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_at_call = Some(1);
    failed.add(b"a");
    assert!(failed.take_flush().is_none());
}

#[test]
fn take_flush_hands_the_collector_over_once() {
    let (mut spend, _state) = collection();
    spend.add(b"a");
    assert!(spend.take_flush().is_some());
    assert!(spend.take_flush().is_none());
    spend.add(b"b");
    assert!(spend.take_flush().is_none());
}

#[test]
fn owned_spend_stops_collecting_exactly_past_the_byte_cap() {
    let config = fake_config();
    let collected = Arc::clone(&config.collected);
    let mut sink = owned_spend(config);
    sink.collected_bytes = MAX_COLLECTED_BYTES.saturating_sub(1);
    PassthroughStreamCollector::add(&mut sink, b"a");
    assert!(!sink.disabled);
    assert_eq!(sink.chunks.len(), 1);
    PassthroughStreamCollector::add(&mut sink, b"b");
    assert!(sink.disabled);
    assert!(sink.chunks.is_empty());
    let mut logging = NoopPassthroughLogging;
    assert!(
        sink.build_logged_response(&mut logging)
            .expect("a disabled collector builds nothing")
            .is_none()
    );
    assert!(
        collected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}
