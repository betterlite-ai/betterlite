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
use crate::stream_decode::{ChunkStream, DecodedChunk, TransportChunk, decode_anthropic_event};

use fixture::{MAX_EVENT_BYTES, Outcome, converter, delta};

#[rstest]
#[case::split_event_data(scenarios::split_event_data)]
#[case::split_unicode(scenarios::split_unicode)]
#[case::multi_line_sse(scenarios::multi_line_sse)]
#[case::done_marker(scenarios::done_marker)]
#[case::invalid_json_event(scenarios::invalid_json_event)]
#[case::native_error_event(scenarios::native_error_event)]
#[case::truncated_final_event(scenarios::truncated_final_event)]
#[case::passthrough_chunk_conversion(scenarios::passthrough_chunk_conversion)]
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
        // A fresh converter starts with an empty buffer, so its count starts at zero.
        let mut accumulated_bytes = 0usize;
        let category = match decode_anthropic_event(
            &mut converter(),
            &payload,
            MAX_EVENT_BYTES,
            &mut accumulated_bytes,
        ) {
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

fn assert_text(outcome: &Outcome, expected: &str) {
    let Ok(DecodedChunk::Event(chunk)) = outcome else {
        panic!("a text event must arrive, got {outcome:?}");
    };
    assert_eq!(chunk.choices[0].delta.content, expected);
}

const DROPPED: &str = "unparsable bytes from the accumulated anthropic stream buffer";

fn drop_warnings(counts: &[usize]) -> Vec<String> {
    counts
        .iter()
        .map(|count| format!("dropping {count} {DROPPED}"))
        .collect()
}

// Every broken fragment here has a valid event glued behind it, so a drop must end where the broken value ends.
#[test]
fn drops_a_corrupted_fragment_and_keeps_later_text_and_usage() {
    let (guard, warnings) = fixture::capture_warnings();
    let outcomes = fixture::drive_sync(vec![
        Ok(TransportChunk::Text(
            r#"data: {"type":}{"type":"ping"}"#.to_owned(),
        )),
        Ok(TransportChunk::Text(format!(
            r#"data: {{"a"}}{}"#,
            delta("v").trim_start_matches("data:")
        ))),
        Ok(TransportChunk::Text(format!(
            r#"data: "bad\q"{}"#,
            delta("u").trim_start_matches("data:")
        ))),
        Ok(TransportChunk::Text(format!(
            "data: nope junk{}",
            delta("z").trim_start_matches("data:")
        ))),
        Ok(TransportChunk::Text(delta("t"))),
        Ok(TransportChunk::Text(
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#
                .to_owned(),
        )),
    ]);
    drop(guard);
    assert_eq!(outcomes.len(), 6);
    assert_text(&outcomes[0], "");
    assert_text(&outcomes[1], "v");
    assert_text(&outcomes[2], "u");
    assert_text(&outcomes[3], "z");
    assert_text(&outcomes[4], "t");
    let Ok(DecodedChunk::Event(usage)) = &outcomes[5] else {
        panic!("the usage event after the corrupted fragments must arrive");
    };
    assert_eq!(usage.usage.as_ref().unwrap().completion_tokens, 7);
    assert_eq!(warnings.messages(), drop_warnings(&[9, 5, 7, 4, 4]));
}

#[test]
fn a_fragment_that_cannot_close_is_not_scanned() {
    let (guard, warnings) = fixture::capture_warnings();
    let outcomes = fixture::drive_sync(vec![
        Ok(TransportChunk::Text("data: nope".to_owned())),
        Ok(TransportChunk::Text(format!(
            "data:junk{}",
            delta("y").trim_start_matches("data:")
        ))),
    ]);
    drop(guard);
    assert_eq!(outcomes.len(), 1);
    assert_text(&outcomes[0], "y");
    assert_eq!(warnings.messages(), drop_warnings(&[8]));
}

#[test]
fn an_unfinished_value_is_kept_while_it_can_still_complete() {
    let (guard, warnings) = fixture::capture_warnings();
    let outcomes = fixture::drive_sync(vec![
        Ok(TransportChunk::Text(
            r#"data:{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"#.to_owned(),
        )),
        Ok(TransportChunk::Text("data:}".to_owned())),
        Ok(TransportChunk::Text(r#"data:"}}"#.to_owned())),
    ]);
    drop(guard);
    assert_eq!(outcomes.len(), 1);
    assert_text(&outcomes[0], "a}");
    assert!(warnings.messages().is_empty());
}

#[test]
fn drops_many_broken_tokens_in_one_line() {
    let (guard, warnings) = fixture::capture_warnings();
    let tokens = "nope ".repeat(64);
    let outcomes = fixture::drive_sync(vec![Ok(TransportChunk::Text(format!(
        "data: {tokens}{}",
        delta("q").trim_start_matches("data:")
    )))]);
    drop(guard);
    assert_eq!(outcomes.len(), 1);
    assert_text(&outcomes[0], "q");
    assert_eq!(warnings.messages(), drop_warnings(&[4; 64]));
}

#[test]
fn a_value_split_over_many_non_closing_data_lines_accumulates() {
    let steps: Vec<fixture::Source> = std::iter::once(Ok(TransportChunk::Text(
        r#"data:{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"#
            .to_owned(),
    )))
    .chain((0..8).map(|_| Ok(TransportChunk::Text(format!("data:{}", "x".repeat(64))))))
    .chain(std::iter::once(Ok(TransportChunk::Text(
        r#"data:"}}"#.to_owned(),
    ))))
    .collect();
    let outcomes = fixture::drive_sync(steps);
    assert_eq!(outcomes.len(), 1);
    assert_text(&outcomes[0], &"x".repeat(512));
}

// The count is the caller's, so a snapshot check reads the empty buffer as 0 and passes where the count trips.
#[test]
fn the_event_bound_reads_the_count_instead_of_materializing_the_buffer() {
    let steps: Vec<fixture::Source> = vec![
        Ok(TransportChunk::Text(format!("data:{}", "x".repeat(64)))),
        Ok(TransportChunk::Text(format!("data:{}", "x".repeat(64)))),
    ];
    let outcomes = ChunkStream {
        converter: converter(),
        source: steps.into_iter(),
        source_ended: false,
        accumulated_bytes: 95,
        max_event_bytes: 100,
    }
    .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 2);
    assert!(matches!(
        &outcomes[0],
        Err(DecodeError::TooLarge { limit: 100 })
    ));
}

// A line can carry several values, so the pop leaves the rest buffered and the next line must count them.
#[test]
fn the_count_after_a_produced_value_is_the_bytes_still_buffered() {
    let line = r#"data:{"type":"ping"}{"type":"ping"}{"type":"ping"}"#;
    let steps: Vec<fixture::Source> = vec![
        Ok(TransportChunk::Text(line.to_owned())),
        Ok(TransportChunk::Text(line.to_owned())),
    ];
    let outcomes = ChunkStream {
        converter: converter(),
        source: steps.into_iter(),
        source_ended: false,
        accumulated_bytes: 0,
        max_event_bytes: 48,
    }
    .collect::<Vec<_>>();
    // The error ends the second line, then the two values the first line left buffered drain one per pull.
    assert_eq!(outcomes.len(), 4);
    assert_text(&outcomes[0], "");
    assert!(matches!(
        &outcomes[1],
        Err(DecodeError::TooLarge { limit: 48 })
    ));
    assert_text(&outcomes[2], "");
    assert_text(&outcomes[3], "");
}

// The buffer outlives one payload, so the caller's count does too: five deferred bytes count against the second call.
#[test]
fn the_entry_bound_carries_the_count_across_calls_on_one_converter() {
    let mut converter = converter();
    let mut accumulated_bytes = 0usize;
    let first = decode_anthropic_event(&mut converter, br#"{"a":"#, 8, &mut accumulated_bytes);
    assert!(matches!(first, Ok(None)));
    assert_eq!(accumulated_bytes, 5);
    let second = decode_anthropic_event(&mut converter, b"12345", 8, &mut accumulated_bytes);
    assert!(matches!(second, Err(DecodeError::TooLarge { limit: 8 })));
}

#[test]
fn drops_done_markers_and_keeps_later_events() {
    let (guard, warnings) = fixture::capture_warnings();
    let glued = format!("data: [DONE]{}", delta("y").trim_start_matches("data:"));
    let outcomes = fixture::drive_sync(vec![
        Ok(TransportChunk::Text("data: [DONE]".to_owned())),
        Ok(TransportChunk::Text(delta("x"))),
        Ok(TransportChunk::Text(
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#
                .to_owned(),
        )),
        Ok(TransportChunk::Text(glued)),
    ]);
    drop(guard);
    assert_eq!(outcomes.len(), 3);
    let Ok(DecodedChunk::Event(text)) = &outcomes[0] else {
        panic!("the text event after the first [DONE] must arrive");
    };
    assert_eq!(text.choices[0].delta.content, "x");
    let Ok(DecodedChunk::Event(usage)) = &outcomes[1] else {
        panic!("the usage event after the first [DONE] must arrive");
    };
    assert_eq!(usage.usage.as_ref().unwrap().completion_tokens, 5);
    let Ok(DecodedChunk::Event(glued_event)) = &outcomes[2] else {
        panic!("the text event glued behind the second [DONE] must arrive");
    };
    assert_eq!(glued_event.choices[0].delta.content, "y");
    assert_eq!(warnings.messages(), drop_warnings(&[6, 6]));
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
        accumulated_bytes: 0,
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
        Ok(TransportChunk::Text("data: 123456789".to_owned())),
    ];
    let kept = ChunkStream {
        converter: converter(),
        source: at_limit.into_iter(),
        source_ended: false,
        accumulated_bytes: 0,
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
        accumulated_bytes: 0,
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
