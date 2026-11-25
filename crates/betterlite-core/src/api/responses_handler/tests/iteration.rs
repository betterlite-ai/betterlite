use std::sync::atomic::Ordering;

use super::support::{
    FakeError, Fixture, async_iterator, completed_chunk, created_chunk, delta_chunk, done_chunk,
    object, sync_iterator,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::responses_handler::error::InteractionsStreamError;
use crate::api::responses_handler::events::completion_event;
use crate::api::responses_handler::state::InteractionsStreamStateError;

// A fixture whose bounds are small enough for one delta to reach them.
fn fixture_with_limits(json_bytes: usize, event_bytes: usize) -> Fixture {
    let mut fixture = Fixture::new("gpt-4o");
    fixture.limits = ApiLimits {
        max_json_bytes: json_bytes,
        max_event_bytes: event_bytes,
        max_items: 64 * 1024,
    };
    fixture
}

fn expected_kinds() -> Vec<String> {
    [
        "interaction.created",
        "step.start",
        "step.delta",
        "step.stop",
        "interaction.completed",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn sync_iterator_drains_the_queue_before_pulling_again() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = sync_iterator(
        &fixture,
        vec![
            Ok(delta_chunk("item_1", "hello")),
            Ok(created_chunk("resp_1")),
        ],
    );
    let created = iterator
        .next_event()
        .expect("the interaction start")
        .expect("the fake stream does not fail");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    let started = iterator
        .next_event()
        .expect("the step start")
        .expect("the fake stream does not fail");
    assert_eq!(started.event_type.as_deref(), Some("step.start"));
    let delta = iterator
        .next_event()
        .expect("the delta")
        .expect("the fake stream does not fail");
    assert_eq!(delta.event_type.as_deref(), Some("step.delta"));
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
}

#[test]
fn sync_iterator_skips_chunks_that_produce_no_events() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = sync_iterator(
        &fixture,
        vec![Ok(done_chunk()), Ok(delta_chunk("item_1", "hi"))],
    );
    let created = iterator
        .next_event()
        .expect("the interaction start")
        .expect("the fake stream does not fail");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
    assert_eq!(pulls.load(Ordering::SeqCst), 2);
}

#[test]
fn sync_iterator_flushes_the_terminal_events_at_eof() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, _pulls) = sync_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    let mut kinds = Vec::new();
    while let Some(step) = iterator.next_event() {
        let event = step.expect("the fake stream does not fail");
        kinds.push(event.event_type.expect("every event names its type"));
    }
    assert_eq!(kinds, expected_kinds());
    assert!(iterator.next_event().is_none());
}

#[test]
fn sync_iterator_stops_after_the_terminal_flush() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = sync_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    while iterator.next_event().is_some() {}
    let drained = pulls.load(Ordering::SeqCst);
    assert!(iterator.next_event().is_none());
    assert!(iterator.next_event().is_none());
    assert_eq!(pulls.load(Ordering::SeqCst), drained);
}

#[test]
fn sync_iterator_returns_the_upstream_error() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, _pulls) = sync_iterator(&fixture, vec![Err(FakeError::Stream)]);
    assert!(matches!(
        iterator.next_event(),
        Some(Err(InteractionsStreamError::Upstream(FakeError::Stream)))
    ));
}

#[test]
fn sync_iterator_stops_at_the_upstream_completion() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = sync_iterator(
        &fixture,
        vec![
            Ok(delta_chunk("item_1", "hello")),
            Ok(completed_chunk("resp_1")),
            Ok(delta_chunk("item_1", "late")),
        ],
    );
    let mut kinds = Vec::new();
    while let Some(step) = iterator.next_event() {
        let event = step.expect("the fake stream does not fail");
        kinds.push(event.event_type.expect("every event names its type"));
    }
    assert_eq!(kinds, expected_kinds());
    assert_eq!(
        pulls.load(Ordering::SeqCst),
        2,
        "the late chunk is never read"
    );
}

#[test]
fn sync_iterator_completes_with_the_collected_text() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, _pulls) = sync_iterator(
        &fixture,
        vec![
            Ok(delta_chunk("item_1", "hell")),
            Ok(delta_chunk("item_1", "o")),
        ],
    );
    let mut last = None;
    while let Some(step) = iterator.next_event() {
        last = Some(step.expect("the fake stream does not fail"));
    }
    let completion = last.expect("the completion is the last event");
    assert_eq!(
        completion.steps,
        Some(vec![object(
            r#"{"type": "model_output", "content": [{"type": "text", "text": "hello"}]}"#
        )])
    );
}

#[test]
fn sync_iterator_keeps_text_at_the_json_bound() {
    let fixture = fixture_with_limits(5, 1024);
    let (mut iterator, _pulls) = sync_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    let created = iterator
        .next_event()
        .expect("the interaction start")
        .expect("five bytes fit a five-byte bound");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
}

#[test]
fn sync_iterator_returns_a_limit_error_past_the_json_bound() {
    let fixture = fixture_with_limits(4, 1024);
    let (mut iterator, _pulls) = sync_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    assert!(matches!(
        iterator.next_event(),
        Some(Err(InteractionsStreamError::State(
            InteractionsStreamStateError::Limit(ApiLimitError::JsonTooLarge {
                observed: 5,
                limit: 4
            })
        )))
    ));
}

#[test]
fn sync_iterator_keeps_the_completion_at_the_event_bound() {
    let completion = completion_event("item_1", "gpt-4o", "hello");
    let encoded = serde_json::to_vec(&completion).expect("the completion encodes");
    let fixture = fixture_with_limits(1024, encoded.len());
    let (mut iterator, _pulls) = sync_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    let created = iterator
        .next_event()
        .expect("the interaction start")
        .expect("the encoded completion fits its own size");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
}

#[test]
fn sync_iterator_returns_a_limit_error_past_the_event_bound() {
    let completion = completion_event("item_1", "gpt-4o", "hello");
    let observed = serde_json::to_vec(&completion)
        .expect("the completion encodes")
        .len();
    let fixture = fixture_with_limits(1024, 8);
    let (mut iterator, _pulls) = sync_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    let Err(InteractionsStreamError::State(InteractionsStreamStateError::Limit(error))) =
        iterator.next_event().expect("the overflow is reported")
    else {
        panic!("an encoded completion past the event bound must be refused");
    };
    assert_eq!(error, ApiLimitError::EventTooLarge { observed, limit: 8 });
}

#[tokio::test]
async fn async_iterator_drains_the_queue_before_pulling_again() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = async_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    let created = iterator
        .next_event()
        .await
        .expect("the interaction start")
        .expect("the fake stream does not fail");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
    let started = iterator
        .next_event()
        .await
        .expect("the step start")
        .expect("the fake stream does not fail");
    assert_eq!(started.event_type.as_deref(), Some("step.start"));
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn async_iterator_skips_chunks_that_produce_no_events() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = async_iterator(
        &fixture,
        vec![Ok(done_chunk()), Ok(delta_chunk("item_1", "hi"))],
    );
    let created = iterator
        .next_event()
        .await
        .expect("the interaction start")
        .expect("the fake stream does not fail");
    assert_eq!(created.event_type.as_deref(), Some("interaction.created"));
    assert_eq!(pulls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn async_iterator_flushes_the_terminal_events_at_eof() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, _pulls) = async_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    let mut kinds = Vec::new();
    while let Some(step) = iterator.next_event().await {
        let event = step.expect("the fake stream does not fail");
        kinds.push(event.event_type.expect("every event names its type"));
    }
    assert_eq!(kinds, expected_kinds());
}

#[tokio::test]
async fn async_iterator_returns_the_upstream_error() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, _pulls) = async_iterator(&fixture, vec![Err(FakeError::Stream)]);
    assert!(matches!(
        iterator.next_event().await,
        Some(Err(InteractionsStreamError::Upstream(FakeError::Stream)))
    ));
}

#[tokio::test]
async fn async_iterator_returns_a_limit_error_past_the_json_bound() {
    let fixture = fixture_with_limits(4, 1024);
    let (mut iterator, _pulls) = async_iterator(&fixture, vec![Ok(delta_chunk("item_1", "hello"))]);
    assert!(matches!(
        iterator.next_event().await,
        Some(Err(InteractionsStreamError::State(
            InteractionsStreamStateError::Limit(ApiLimitError::JsonTooLarge {
                observed: 5,
                limit: 4
            })
        )))
    ));
}

#[tokio::test]
async fn async_iterator_stops_at_the_upstream_completion() {
    let fixture = Fixture::new("gpt-4o");
    let (mut iterator, pulls) = async_iterator(
        &fixture,
        vec![
            Ok(delta_chunk("item_1", "hello")),
            Ok(completed_chunk("resp_1")),
            Ok(delta_chunk("item_1", "late")),
        ],
    );
    while iterator.next_event().await.is_some() {}
    assert_eq!(pulls.load(Ordering::SeqCst), 2);
}
