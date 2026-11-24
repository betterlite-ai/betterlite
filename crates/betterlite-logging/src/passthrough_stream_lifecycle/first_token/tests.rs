#![cfg(test)]

use indexmap::IndexMap;

use super::stamp_first_chunk_if_needed;
use crate::passthrough_stream_lifecycle::support::{
    StepClock, completion_start_time, model_details, object, time,
};

#[test]
fn the_first_chunk_sets_the_completion_start_time() {
    let logging_obj = object(&[], IndexMap::new());
    let clock = StepClock::new(&["2024-01-01T00:00:01+00:00"]);
    stamp_first_chunk_if_needed(logging_obj.as_ref(), clock.as_ref());
    assert_eq!(
        completion_start_time(logging_obj.as_ref()).as_deref(),
        Some("2024-01-01T00:00:01+00:00")
    );
    assert_eq!(
        model_details(logging_obj.as_ref())
            .get("completion_start_time")
            .cloned(),
        Some(serde_json::from_str("\"2024-01-01T00:00:01+00:00\"").unwrap())
    );
}

#[test]
fn later_chunks_keep_the_first_completion_start_time() {
    let logging_obj = object(&[], IndexMap::new());
    let clock = StepClock::new(&["2024-01-01T00:00:01+00:00", "2024-01-01T00:00:05+00:00"]);
    stamp_first_chunk_if_needed(logging_obj.as_ref(), clock.as_ref());
    stamp_first_chunk_if_needed(logging_obj.as_ref(), clock.as_ref());
    assert_eq!(
        completion_start_time(logging_obj.as_ref()).as_deref(),
        Some("2024-01-01T00:00:01+00:00")
    );
    assert_eq!(clock.calls(), 1);
}

#[test]
fn a_stamped_stream_never_reads_the_clock() {
    let logging_obj = object(&[], IndexMap::new());
    logging_obj.edit(&mut |state| {
        state.completion_start_time = Some(time("2023-12-31T23:59:59+00:00"));
    });
    let clock = StepClock::new(&["2024-01-01T00:00:01+00:00"]);
    stamp_first_chunk_if_needed(logging_obj.as_ref(), clock.as_ref());
    assert_eq!(
        completion_start_time(logging_obj.as_ref()).as_deref(),
        Some("2023-12-31T23:59:59+00:00")
    );
    assert_eq!(clock.calls(), 0);
}

#[test]
fn an_empty_logging_slot_does_not_panic() {
    let logging_obj = object(&[], IndexMap::new());
    drop(logging_obj.slot.lock().unwrap().take());
    let clock = StepClock::new(&["2024-01-01T00:00:01+00:00"]);
    stamp_first_chunk_if_needed(logging_obj.as_ref(), clock.as_ref());
    assert_eq!(clock.calls(), 0);
}
