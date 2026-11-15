#![cfg(test)]

mod fixture;
mod scenarios;

use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Context;

use futures_util::{Stream as _, stream, task::noop_waker};
use rstest::rstest;

use crate::stream_decode::error::DecodeError;
use crate::stream_decode::{ChunkStream, TransportChunk, decode_anthropic_event};

use fixture::{MAX_EVENT_BYTES, converter};

#[rstest]
#[case::case_1_split_event_data(scenarios::split_event_data)]
#[case::case_2_split_unicode(scenarios::split_unicode)]
#[case::case_3_multi_line_sse(scenarios::multi_line_sse)]
#[case::case_4_done_marker(scenarios::done_marker)]
#[case::case_5_invalid_json_event(scenarios::invalid_json_event)]
#[case::case_6_native_error_event(scenarios::native_error_event)]
#[case::case_7_truncated_final_event(scenarios::truncated_final_event)]
#[case::case_8_passthrough_chunk_conversion(scenarios::passthrough_chunk_conversion)]
fn stream_decode_cases(#[case] scenario: fn()) {
    scenario();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Value,
    Pending,
    Failed,
}

fn corpus_outcomes() -> Vec<(String, Category)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_stream_decode");
    let mut outcomes = Vec::new();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let payload = fs::read(&path).unwrap();
        let category = match decode_anthropic_event(&mut converter(), &payload, MAX_EVENT_BYTES) {
            Ok(Some(_)) => Category::Value,
            Ok(None) => Category::Pending,
            Err(_) => Category::Failed,
        };
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        outcomes.push((name, category));
    }
    outcomes
}

fn expected_category(name: &str) -> Option<Category> {
    if name.starts_with("valid") {
        return Some(Category::Value);
    }
    if name.starts_with("truncated") {
        return Some(Category::Pending);
    }
    if name.starts_with("malformed") {
        return Some(Category::Failed);
    }
    None
}

#[test]
fn replays_the_anthropic_stream_decode_corpus() {
    let outcomes = corpus_outcomes();
    assert!(!outcomes.is_empty());
    let mismatched = outcomes
        .iter()
        .filter(|(name, category)| {
            expected_category(name).is_some_and(|expected| expected != *category)
        })
        .collect::<Vec<_>>();
    assert!(mismatched.is_empty(), "{mismatched:?}");
    assert!(
        outcomes
            .iter()
            .any(|(name, category)| name.starts_with("boundary") && *category == Category::Value)
    );
    assert!(
        outcomes
            .iter()
            .any(|(name, category)| name.starts_with("boundary") && *category == Category::Pending)
    );
    assert!(
        outcomes
            .iter()
            .any(|(name, category)| name.starts_with("boundary") && *category == Category::Failed)
    );
}

#[test]
fn rejects_an_accumulated_event_over_max_event_bytes() {
    let over_limit: Vec<fixture::Source> = vec![
        Ok(TransportChunk::Text(r#"data: {"a":"#.to_owned())),
        Ok(TransportChunk::Text("data: 0123456789".to_owned())),
    ];
    let outcomes = ChunkStream {
        converter: converter(),
        source: over_limit.into_iter(),
        source_ended: false,
        max_event_bytes: 16,
    }
    .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 1);
    let Err(DecodeError::TooLarge { limit }) = &outcomes[0] else {
        panic!("an over-limit accumulation must be rejected");
    };
    assert_eq!(*limit, 16);
    // Exactly at the limit the fragment stays buffered and the stream ends without an item.
    let at_limit: Vec<fixture::Source> = vec![
        Ok(TransportChunk::Text(r#"data: {"a":"#.to_owned())),
        Ok(TransportChunk::Text("data: 12345678".to_owned())),
    ];
    let kept = ChunkStream {
        converter: converter(),
        source: at_limit.into_iter(),
        source_ended: false,
        max_event_bytes: 16,
    }
    .collect::<Vec<_>>();
    assert!(kept.is_empty());
}

#[test]
fn dropping_the_stream_while_pending_keeps_the_source_unpulled() {
    let yielded = Rc::new(Cell::new(0));
    let dropped = Rc::new(Cell::new(false));
    let items: Vec<fixture::Source> = vec![
        Ok(TransportChunk::Text(r#"data: {"a":"#.to_owned())),
        Ok(TransportChunk::Text("data: 1}".to_owned())),
    ];
    let mut stream = ChunkStream {
        converter: converter(),
        source: fixture::Tracked {
            inner: fixture::PendFirst {
                inner: stream::iter(items),
                pending: false,
            },
            yielded: Rc::clone(&yielded),
            dropped: Rc::clone(&dropped),
        },
        source_ended: false,
        max_event_bytes: MAX_EVENT_BYTES,
    };
    let waker = noop_waker();
    let mut context = Context::from_waker(&waker);
    assert!(Pin::new(&mut stream).poll_next(&mut context).is_pending());
    assert!(Pin::new(&mut stream).poll_next(&mut context).is_pending());
    assert_eq!(yielded.get(), 1);
    assert!(!dropped.get());
    drop(stream);
    assert!(dropped.get());
    assert_eq!(yielded.get(), 1);
}
