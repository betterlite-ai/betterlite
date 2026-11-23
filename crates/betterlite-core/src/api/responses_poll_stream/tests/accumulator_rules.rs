use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support;
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::responses_poll_stream::accumulator::{AccumulatorLimitError, OutputAccumulator};
use crate::api::responses_poll_stream::clock::PollClock;
use crate::api::responses_poll_stream::event::{Item, PollEvent, PollEventError};

fn event(source: &str) -> PollEvent {
    PollEvent::decode(&support::json(source)).expect("the event decodes")
}

fn forced(accumulator: &mut OutputAccumulator, clock: &dyn PollClock) -> Json {
    accumulator
        .flush_forced(clock)
        .expect("dirty output flushes")
}

fn output_ids(output: &Json) -> Vec<String> {
    let Json::List(items) = output else {
        panic!("the output is a list");
    };
    items.iter().map(|item| support::text(item, "id")).collect()
}

fn content(output: &Json) -> Vec<Json> {
    let Json::List(items) = output else {
        panic!("the output is a list");
    };
    let Some(first) = items.first() else {
        panic!("the output has an item");
    };
    let Json::List(parts) = support::field(first, "content") else {
        panic!("the item has a content list");
    };
    parts
}

const ADDED_M1: &str =
    r#"{"item":{"content":[],"id":"m1","marker":"first"},"type":"response.output_item.added"}"#;
const ADDED_M2: &str =
    r#"{"item":{"content":[],"id":"m2","marker":"second"},"type":"response.output_item.added"}"#;
const REPLACED_M1: &str =
    r#"{"item":{"content":[],"id":"m1","marker":"replaced"},"type":"response.output_item.done"}"#;
const PART_A: &str = r#"{"item_id":"m1","part":{"text":"a","type":"output_text"},"type":"response.content_part.added"}"#;
const PART_B: &str = r#"{"item_id":"m1","part":{"text":"b","type":"output_text"},"type":"response.content_part.added"}"#;
const PART_EMPTY: &str = r#"{"item_id":"m1","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;
const PART_DONE_C: &str = r#"{"content_index":1,"item_id":"m1","part":{"text":"c","type":"output_text"},"type":"response.content_part.done"}"#;

pub(super) fn duplicate_ids_keep_insertion_order() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    accumulator
        .apply(event(ADDED_M2))
        .expect("the item applies");
    accumulator
        .apply(event(REPLACED_M1))
        .expect("the item applies");
    let output = forced(&mut accumulator, &clock);
    assert_eq!(
        output_ids(&output),
        vec![String::from("m1"), String::from("m2")]
    );
    let Json::List(items) = &output else {
        panic!("the output is a list");
    };
    let Some(first) = items.first() else {
        panic!("the output has an item");
    };
    assert_eq!(support::text(first, "marker"), "replaced");
}

pub(super) fn missing_ids_are_ignored() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let sources = [
        r#"{"item":{"content":[],"type":"message"},"type":"response.output_item.added"}"#,
        r#"{"content_index":0,"delta":"x","item_id":"absent","type":"response.output_text.delta"}"#,
        r#"{"content_index":0,"delta":"x","type":"response.output_text.delta"}"#,
        r#"{"item_id":"absent","part":{"text":"x"},"type":"response.content_part.added"}"#,
    ];
    for source in sources {
        accumulator.apply(event(source)).expect("the event applies");
    }
    assert!(accumulator.flush_forced(&clock).is_none());
}

pub(super) fn part_done_replaces_selected_part() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    // the flush before each event clears the earlier dirt, so every flush below proves that event dirtied
    let before = content(&forced(&mut accumulator, &clock));
    assert!(before.is_empty());
    accumulator.apply(event(PART_A)).expect("the part applies");
    let added_a = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&added_a[0], "text"), "a");
    accumulator.apply(event(PART_B)).expect("the part applies");
    let added_b = content(&forced(&mut accumulator, &clock));
    assert_eq!(added_b.len(), 2);
    accumulator
        .apply(event(PART_DONE_C))
        .expect("the part applies");
    let done = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&done[0], "text"), "a");
    assert_eq!(support::text(&done[1], "text"), "c");
}

pub(super) fn part_done_negative_index_keeps_parts() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    let sources = [
        r#"{"item_id":"m1","part":{"text":"a","type":"output_text"},"type":"response.content_part.added"}"#,
        r#"{"content_index":-1,"item_id":"m1","part":{"text":"c","type":"output_text"},"type":"response.content_part.done"}"#,
    ];
    for source in sources {
        accumulator.apply(event(source)).expect("the event applies");
    }
    let parts = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&parts[0], "text"), "a");
}

pub(super) fn delta_negative_index_updates_last_part() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let added = r#"{"item":{"content":[{"text":"a","type":"output_text"},{"text":"b","type":"output_text"}],"id":"m1"},"type":"response.output_item.added"}"#;
    accumulator.apply(event(added)).expect("the item applies");
    let delta =
        r#"{"content_index":-1,"delta":"x","item_id":"m1","type":"response.output_text.delta"}"#;
    accumulator.apply(event(delta)).expect("the delta applies");
    let parts = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&parts[0], "text"), "a");
    assert_eq!(support::text(&parts[1], "text"), "x");
}

pub(super) fn delta_out_of_range_marks_dirty() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let added = r#"{"item":{"content":[{"text":"a","type":"output_text"}],"id":"m1"},"type":"response.output_item.added"}"#;
    accumulator.apply(event(added)).expect("the item applies");
    let before = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&before[0], "text"), "a");
    let delta =
        r#"{"content_index":3,"delta":"x","item_id":"m1","type":"response.output_text.delta"}"#;
    accumulator.apply(event(delta)).expect("the delta applies");
    // the item's dirt is already flushed, so this flush happens only if the delta marked the state
    let after = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&after[0], "text"), "a");
}

#[test]
fn empty_ids_are_used_as_given() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let added =
        r#"{"item":{"content":[],"id":"","type":"message"},"type":"response.output_item.added"}"#;
    accumulator.apply(event(added)).expect("the item applies");
    assert_eq!(
        output_ids(&forced(&mut accumulator, &clock)),
        vec![String::new()]
    );
}

#[test]
fn empty_terminal_status_is_used_as_given() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let terminal = r#"{"response":{"status":""},"type":"response.completed"}"#;
    accumulator
        .apply(event(terminal))
        .expect("the terminal applies");
    assert_eq!(accumulator.final_status(), "");
}

#[test]
fn delta_beyond_the_first_part_errors() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let added = r#"{"item":{"content":[{"text":"a","type":"output_text"}],"id":"m1"},"type":"response.output_item.added"}"#;
    accumulator.apply(event(added)).expect("the item applies");
    let delta =
        r#"{"content_index":-2,"delta":"x","item_id":"m1","type":"response.output_text.delta"}"#;
    let error = accumulator
        .apply(event(delta))
        .expect_err("the negative index falls before the first part");
    assert_eq!(error, PollEventError::ContentIndex { index: -2 });
}

pub(super) fn delta_text_is_keyed_by_item_and_part() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    accumulator
        .apply(event(PART_EMPTY))
        .expect("the first part applies");
    accumulator
        .apply(event(PART_EMPTY))
        .expect("the second part applies");
    let added = content(&forced(&mut accumulator, &clock));
    assert_eq!(added.len(), 2);
    let sources = [
        r#"{"content_index":0,"delta":"A","item_id":"m1","type":"response.output_text.delta"}"#,
        r#"{"content_index":1,"delta":"B","item_id":"m1","type":"response.output_text.delta"}"#,
        r#"{"content_index":0,"delta":"C","item_id":"m1","type":"response.output_text.delta"}"#,
    ];
    for source in sources {
        accumulator.apply(event(source)).expect("the delta applies");
    }
    let parts = content(&forced(&mut accumulator, &clock));
    assert_eq!(support::text(&parts[0], "text"), "AC");
    assert_eq!(support::text(&parts[1], "text"), "B");
}

pub(super) fn item_done_replaces_item() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    accumulator
        .apply(event(REPLACED_M1))
        .expect("the item applies");
    let Json::List(items) = forced(&mut accumulator, &clock) else {
        panic!("the output is a list");
    };
    let Some(first) = items.first() else {
        panic!("the output has an item");
    };
    assert_eq!(support::text(first, "marker"), "replaced");
}

#[test]
fn terminal_output_merges_by_id() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    assert_eq!(
        output_ids(&forced(&mut accumulator, &clock)),
        vec![String::from("m1")]
    );
    let terminal = r#"{"response":{"output":[{"content":[],"id":"m2"}],"status":"completed"},"type":"response.completed"}"#;
    accumulator
        .apply(event(terminal))
        .expect("the terminal applies");
    // the item's dirt is already flushed, so this flush happens only if the terminal merge marked the state
    assert_eq!(
        output_ids(&forced(&mut accumulator, &clock)),
        vec![String::from("m1"), String::from("m2")]
    );
}

#[test]
fn empty_terminal_output_keeps_items() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    let terminal = r#"{"response":{"output":[],"status":"completed"},"type":"response.completed"}"#;
    accumulator
        .apply(event(terminal))
        .expect("the terminal applies");
    assert_eq!(
        output_ids(&forced(&mut accumulator, &clock)),
        vec![String::from("m1")]
    );
}

#[test]
fn absent_terminal_output_is_clean() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let terminal = r#"{"response":{"status":"completed"},"type":"response.completed"}"#;
    accumulator
        .apply(event(terminal))
        .expect("the terminal applies");
    assert!(accumulator.flush_forced(&clock).is_none());
    assert_eq!(accumulator.final_status(), "completed");
}

#[rstest]
#[case::completed("response.completed", "completed")]
#[case::failed("response.failed", "failed")]
#[case::incomplete("response.incomplete", "incomplete")]
#[case::cancelled("response.cancelled", "cancelled")]
fn terminal_event_defaults_status(#[case] kind: &str, #[case] expected: &str) {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let source = format!(r#"{{"type":"{kind}"}}"#);
    accumulator
        .apply(event(&source))
        .expect("the terminal applies");
    let patch = accumulator.final_patch();
    assert_eq!(support::text(&Json::Object(patch.0), "status"), expected);
}

#[test]
fn explicit_terminal_status_wins() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let terminal = r#"{"response":{"status":"queued"},"type":"response.completed"}"#;
    accumulator
        .apply(event(terminal))
        .expect("the terminal applies");
    let patch = accumulator.final_patch();
    assert_eq!(support::text(&Json::Object(patch.0), "status"), "queued");
}

#[test]
fn failed_error_is_captured() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let failed =
        r#"{"response":{"error":{"code":"x"},"status":"failed"},"type":"response.failed"}"#;
    accumulator
        .apply(event(failed))
        .expect("the terminal applies");
    let patch = accumulator.final_patch();
    assert_eq!(
        support::field(&Json::Object(patch.0), "error"),
        support::json(r#"{"code":"x"}"#)
    );
}

#[test]
fn completed_error_is_not_captured() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let failed =
        r#"{"response":{"error":{"code":"x"},"status":"failed"},"type":"response.failed"}"#;
    accumulator
        .apply(event(failed))
        .expect("the terminal applies");
    let completed =
        r#"{"response":{"error":{"code":"y"},"status":"completed"},"type":"response.completed"}"#;
    accumulator
        .apply(event(completed))
        .expect("the terminal applies");
    let patch = accumulator.final_patch();
    assert_eq!(
        support::field(&Json::Object(patch.0), "error"),
        support::json(r#"{"code":"x"}"#)
    );
}

#[test]
fn failed_without_error_clears_it() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let failed =
        r#"{"response":{"error":{"code":"x"},"status":"failed"},"type":"response.failed"}"#;
    accumulator
        .apply(event(failed))
        .expect("the terminal applies");
    let cleared = r#"{"response":{"status":"failed"},"type":"response.failed"}"#;
    accumulator
        .apply(event(cleared))
        .expect("the terminal applies");
    let patch = accumulator.final_patch();
    assert_eq!(support::field(&Json::Object(patch.0), "error"), Json::Null);
}

#[test]
fn later_terminal_replaces_fields() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let first = r#"{"response":{"model":"a","usage":{"input_tokens":1},"status":"completed"},"type":"response.completed"}"#;
    accumulator
        .apply(event(first))
        .expect("the terminal applies");
    let second = r#"{"response":{"model":"b","status":"completed"},"type":"response.completed"}"#;
    accumulator
        .apply(event(second))
        .expect("the terminal applies");
    let patch = Json::Object(accumulator.final_patch().0);
    assert_eq!(support::text(&patch, "model"), "b");
    assert_eq!(support::field(&patch, "usage"), Json::Null);
}

#[test]
fn terminal_fields_reach_patch() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let terminal = r#"{"response":{"incomplete_details":{},"parallel_tool_calls":false,"status":"completed","temperature":0,"tools":[],"usage":{"input_tokens":0}},"type":"response.completed"}"#;
    accumulator
        .apply(event(terminal))
        .expect("the terminal applies");
    let patch = Json::Object(accumulator.final_patch().0);
    assert_eq!(
        support::field(&patch, "usage"),
        support::json(r#"{"input_tokens":0}"#)
    );
    assert_eq!(support::field(&patch, "temperature"), support::json("0"));
    assert_eq!(support::field(&patch, "tools"), support::json("[]"));
    assert_eq!(
        support::field(&patch, "parallel_tool_calls"),
        Json::Bool(false)
    );
    assert_eq!(
        support::field(&patch, "incomplete_details"),
        support::json("{}")
    );
}

#[test]
fn in_progress_decodes_to_the_in_progress_event() {
    let source = r#"{"response":{"status":"in_progress"},"type":"response.in_progress"}"#;
    assert_eq!(event(source), PollEvent::InProgress);
}

#[rstest]
#[case::in_progress(r#"{"response":{"status":"in_progress"},"type":"response.in_progress"}"#)]
#[case::unknown(r#"{"type":"response.something_else"}"#)]
fn in_progress_and_unknown_events_do_not_dirty(#[case] source: &str) {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator.apply(event(source)).expect("the event applies");
    assert!(accumulator.flush_forced(&clock).is_none());
}

#[test]
fn absent_optional_keys_take_the_source_defaults() {
    assert_eq!(event("{}"), PollEvent::Unknown);
    let PollEvent::OutputItemAdded(item) = event(r#"{"type":"response.output_item.added"}"#) else {
        panic!("the delivered item decodes");
    };
    assert_eq!(item.id, None);
    assert!(item.value.is_empty());
    let PollEvent::ContentPartAdded(item_id, part) =
        event(r#"{"type":"response.content_part.added"}"#)
    else {
        panic!("the added part decodes");
    };
    assert_eq!(item_id, None);
    assert_eq!(part, support::json("{}"));
    let PollEvent::ContentPartDone(done_item_id, done_part, done_index) =
        event(r#"{"type":"response.content_part.done"}"#)
    else {
        panic!("the done part decodes");
    };
    assert_eq!(done_item_id, None);
    assert_eq!(done_part, support::json("{}"));
    assert_eq!(done_index, 0);
    let PollEvent::OutputTextDelta(delta_item_id, delta_index, delta) =
        event(r#"{"type":"response.output_text.delta"}"#)
    else {
        panic!("the delta decodes");
    };
    assert_eq!(delta_item_id, None);
    assert_eq!(delta_index, 0);
    assert_eq!(delta, "");
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(r#"{"type":"response.completed"}"#))
        .expect("the terminal applies");
    let patch = Json::Object(accumulator.final_patch().0);
    assert_eq!(support::text(&patch, "status"), "completed");
    assert_eq!(support::field(&patch, "usage"), Json::Null);
}

#[test]
fn items_stop_at_the_items_bound() {
    let limits = ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 2,
    };
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    let first = Item {
        id: Some(String::from("m1")),
        value: IndexMap::new(),
    };
    accumulator
        .apply(PollEvent::OutputItemAdded(first))
        .expect("the item applies");
    let second = Item {
        id: Some(String::from("m2")),
        value: IndexMap::new(),
    };
    accumulator
        .apply(PollEvent::OutputItemAdded(second))
        .expect("the item applies");
    accumulator
        .check_limits(limits)
        .expect("the item count is at the limit");
    let over = Item {
        id: Some(String::from("m3")),
        value: IndexMap::new(),
    };
    accumulator
        .apply(PollEvent::OutputItemAdded(over))
        .expect("the item applies");
    let error = accumulator
        .check_limits(limits)
        .expect_err("the item past the limit is refused");
    let AccumulatorLimitError::Limit(ApiLimitError::TooManyItems { observed, limit }) = error
    else {
        panic!("the item bound rejects the third item");
    };
    assert_eq!((observed, limit), (3_usize, 2_usize));
}

#[test]
fn delta_text_stops_at_the_json_bound() {
    let clock = support::clock(&[0]);
    let mut accumulator = OutputAccumulator::started(&clock);
    accumulator
        .apply(event(ADDED_M1))
        .expect("the item applies");
    accumulator
        .apply(event(PART_EMPTY))
        .expect("the part applies");
    let id = Some(String::from("m1"));
    accumulator
        .apply(PollEvent::OutputTextDelta(id.clone(), 0, "x".repeat(1024)))
        .expect("the delta applies");
    let output = forced(&mut accumulator, &clock);
    let values = Json::List(vec![output, Json::Text(String::from("completed"))]);
    let bytes = serde_json::to_vec(&values).expect("the measured output encodes");
    let observed = bytes.len();
    let limits = ApiLimits {
        max_json_bytes: observed,
        max_event_bytes: 16 * 1024 * 1024,
        max_items: 64 * 1024,
    };
    accumulator
        .check_limits(limits)
        .expect("the text is exactly at the limit");
    accumulator
        .apply(PollEvent::OutputTextDelta(id, 0, String::from("x")))
        .expect("the delta applies");
    let error = accumulator
        .check_limits(limits)
        .expect_err("the byte past the limit is refused");
    let AccumulatorLimitError::Limit(ApiLimitError::JsonTooLarge {
        observed: rejected,
        limit,
    }) = error
    else {
        panic!("the JSON bound rejects the extra byte");
    };
    assert_eq!((rejected, limit), (observed.saturating_add(1), observed));
}

#[test]
fn reducer_rules() {
    duplicate_ids_keep_insertion_order();
    missing_ids_are_ignored();
    part_done_replaces_selected_part();
    part_done_negative_index_keeps_parts();
    delta_negative_index_updates_last_part();
    delta_out_of_range_marks_dirty();
    delta_text_is_keyed_by_item_and_part();
    item_done_replaces_item();
}
