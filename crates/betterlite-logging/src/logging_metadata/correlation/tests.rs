#![cfg(test)]

use std::time::{Duration, Instant};

use rstest::rstest;

use super::TraceIdCache;
use super::TraceService;
use super::logging_payload_session_id;
use super::logging_payload_trace_id;
use crate::logging_metadata::params::LitellmParams;

fn params(encoded: &str) -> LitellmParams {
    serde_json::from_str(encoded).expect("params decode")
}

#[rstest]
#[case::session_first_off(
    "{\"litellm_session_id\": \"session\", \"litellm_trace_id\": \"trace\"}",
    false,
    "session"
)]
#[case::trace_first_on(
    "{\"litellm_session_id\": \"session\", \"litellm_trace_id\": \"trace\"}",
    true,
    "trace"
)]
#[case::metadata_session_off(
    "{\"metadata\": {\"session_id\": \"m-session\", \"trace_id\": \"m-trace\"}}",
    false,
    "m-session"
)]
#[case::metadata_trace_off("{\"metadata\": {\"trace_id\": \"m-trace\"}}", false, "m-trace")]
#[case::metadata_trace_on(
    "{\"metadata\": {\"session_id\": \"m-session\", \"trace_id\": \"m-trace\"}}",
    true,
    "m-trace"
)]
#[case::fallback("{}", false, "generated")]
fn logging_metadata_behavior_tests_behavior(
    #[case] encoded: &str,
    #[case] correlation: bool,
    #[case] expected: &str,
) {
    let trace_id = logging_payload_trace_id(&params(encoded), "generated", correlation);
    assert_eq!(trace_id, expected);
}

#[rstest]
#[case::off_is_empty("{\"litellm_session_id\": \"session\"}", false, "")]
#[case::dynamic_session("{\"litellm_session_id\": \"session\"}", true, "session")]
#[case::metadata_session("{\"metadata\": {\"session_id\": \"m-session\"}}", true, "m-session")]
#[case::fallback("{}", true, "generated-session")]
fn session_resolution_reads_the_dynamic_then_metadata_id(
    #[case] encoded: &str,
    #[case] correlation: bool,
    #[case] expected: &str,
) {
    let session_id = logging_payload_session_id(&params(encoded), "generated-session", correlation);
    assert_eq!(session_id, expected);
}

fn bounded(capacity: usize) -> TraceIdCache {
    TraceIdCache::new(capacity, Duration::from_secs(600))
}

#[test]
fn a_recorded_trace_id_is_found_by_its_call_and_service() {
    let now = Instant::now();
    let mut cache = bounded(200);
    cache.record("call-1", TraceService::Langfuse, "trace-1".to_owned(), now);
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse, now),
        Some("trace-1")
    );
    assert_eq!(
        cache.logged_trace_id("call-2", TraceService::Langfuse, now),
        None
    );
}

#[test]
fn a_later_record_replaces_the_trace_id_for_its_call() {
    let now = Instant::now();
    let mut cache = bounded(200);
    cache.record("call-1", TraceService::Langfuse, "trace-1".to_owned(), now);
    cache.record("call-1", TraceService::Langfuse, "trace-2".to_owned(), now);
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse, now),
        Some("trace-2")
    );
}

#[test]
fn the_oldest_entry_is_evicted_at_the_capacity_limit() {
    let now = Instant::now();
    let mut cache = bounded(2);
    cache.record("call-1", TraceService::Langfuse, "trace-1".to_owned(), now);
    cache.record("call-2", TraceService::Langfuse, "trace-2".to_owned(), now);
    cache.record("call-3", TraceService::Langfuse, "trace-3".to_owned(), now);
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse, now),
        None
    );
    assert_eq!(
        cache.logged_trace_id("call-2", TraceService::Langfuse, now),
        Some("trace-2")
    );
    assert_eq!(
        cache.logged_trace_id("call-3", TraceService::Langfuse, now),
        Some("trace-3")
    );
}

#[test]
fn an_entry_expires_after_the_ttl_and_is_dropped_on_the_next_record() {
    let start = Instant::now();
    let mut cache = bounded(2);
    cache.record(
        "call-1",
        TraceService::Langfuse,
        "trace-1".to_owned(),
        start,
    );
    let edge = start + Duration::from_secs(600);
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse, edge),
        Some("trace-1")
    );
    let late = edge + Duration::from_millis(1);
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse, late),
        None
    );
    cache.record("call-2", TraceService::Langfuse, "trace-2".to_owned(), late);
    cache.record("call-3", TraceService::Langfuse, "trace-3".to_owned(), late);
    assert_eq!(
        cache.logged_trace_id("call-2", TraceService::Langfuse, late),
        Some("trace-2")
    );
}

#[test]
fn rewriting_a_live_key_keeps_its_original_expiry() {
    let start = Instant::now();
    let mut cache = bounded(2);
    cache.record(
        "call-1",
        TraceService::Langfuse,
        "trace-1".to_owned(),
        start,
    );
    let middle = start + Duration::from_secs(300);
    cache.record(
        "call-1",
        TraceService::Langfuse,
        "trace-2".to_owned(),
        middle,
    );
    let late = start + Duration::from_secs(601);
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse, late),
        None
    );
}

#[test]
fn empty_strings_are_values_the_caller_chose() {
    let record = params("{\"litellm_trace_id\": \"\", \"metadata\": {\"trace_id\": \"m-trace\"}}");
    assert_eq!(logging_payload_trace_id(&record, "generated", true), "");
}

#[test]
fn a_numeric_metadata_id_is_used_as_its_decimal_text() {
    let record = params("{\"metadata\": {\"trace_id\": 7, \"session_id\": 8}}");
    assert_eq!(logging_payload_trace_id(&record, "generated", true), "7");
    assert_eq!(logging_payload_session_id(&record, "generated", true), "8");
}

#[test]
fn a_record_drops_entries_that_reached_the_ttl_and_keeps_younger_ones() {
    let start = Instant::now();
    let mut cache = bounded(5);
    cache.record("old", TraceService::Langfuse, "trace-old".to_owned(), start);
    let middle = start + Duration::from_secs(300);
    cache.record(
        "young",
        TraceService::Langfuse,
        "trace-young".to_owned(),
        middle,
    );
    let edge = start + Duration::from_secs(600);
    cache.record("new", TraceService::Langfuse, "trace-new".to_owned(), edge);
    assert_eq!(
        cache.logged_trace_id("old", TraceService::Langfuse, edge),
        None
    );
    assert_eq!(
        cache.logged_trace_id("young", TraceService::Langfuse, edge),
        Some("trace-young")
    );
}

#[test]
fn a_boolean_or_list_metadata_id_is_ignored() {
    let record = params("{\"metadata\": {\"trace_id\": true, \"session_id\": [1]}}");
    assert_eq!(
        logging_payload_trace_id(&record, "generated", true),
        "generated"
    );
    assert_eq!(
        logging_payload_session_id(&record, "generated", true),
        "generated"
    );
}
