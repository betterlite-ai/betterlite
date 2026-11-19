#![cfg(test)]

use betterlite_compression::compress::CompressionSkippedReason;
use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::Level;

use super::support::{
    NOW, applied, cache as content, capture, compressed_output, fixed_clock, harness, manual_clock,
    number, output, request, skipped_output, text,
};
use crate::api::compression_interception::hook::resolve_call_id;

fn stored_harness() -> super::support::Harness {
    harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![
            Ok(compressed_output(content(&[("key", "value")]))),
            Ok(skipped_output(CompressionSkippedReason::BelowTrigger, 5)),
        ],
    )
}

#[tokio::test]
async fn a_pre_call_prunes_an_entry_older_than_the_ttl() {
    let (clock, now) = manual_clock(100.0);
    let harness = harness(
        Settings::default(),
        clock,
        vec![
            Ok(compressed_output(content(&[("key", "value")]))),
            Ok(skipped_output(CompressionSkippedReason::BelowTrigger, 5)),
        ],
    );
    let mut first = request();
    first.insert("litellm_call_id".to_owned(), text("call-1"));
    applied(&harness, &mut first).await;
    assert!(harness.interception.cache.entry("call-1").is_some());
    *now.lock().expect("the clock lock") = 1001.0;
    let mut second = request();
    applied(&harness, &mut second).await;
    assert_eq!(harness.interception.cache.entry("call-1"), None);
}

#[tokio::test]
async fn a_pre_call_keeps_an_entry_exactly_the_ttl_old() {
    let (clock, now) = manual_clock(100.0);
    let harness = harness(
        Settings::default(),
        clock,
        vec![
            Ok(compressed_output(content(&[("key", "value")]))),
            Ok(skipped_output(CompressionSkippedReason::BelowTrigger, 5)),
        ],
    );
    let mut first = request();
    first.insert("litellm_call_id".to_owned(), text("call-1"));
    applied(&harness, &mut first).await;
    *now.lock().expect("the clock lock") = 1000.0;
    let mut second = request();
    applied(&harness, &mut second).await;
    assert!(harness.interception.cache.entry("call-1").is_some());
}

#[tokio::test]
async fn get_cache_returns_the_stored_content() {
    let harness = stored_harness();
    let mut body = request();
    body.insert("litellm_call_id".to_owned(), text("call-1"));
    applied(&harness, &mut body).await;
    assert_eq!(
        harness.interception.get_cache(Some("call-1")),
        content(&[("key", "value")])
    );
}

#[rstest]
#[case::absent(None)]
#[case::unknown(Some("unknown"))]
fn get_cache_returns_nothing_without_a_matching_entry(#[case] call_id: Option<&str>) {
    let harness = harness(Settings::default(), fixed_clock(NOW), Vec::new());
    assert!(harness.interception.get_cache(call_id).is_empty());
}

fn body_with(call_id: Option<Json>) -> IndexMap<String, Json> {
    let mut body = request();
    if let Some(call_id) = call_id {
        body.insert("litellm_call_id".to_owned(), call_id);
    }
    body
}

#[rstest]
#[case::logging_object_wins(Some("log-1"), Some(text("kw-1")), Some("log-1"))]
#[case::empty_logging_id_falls_back(Some(""), Some(text("kw-1")), Some("kw-1"))]
#[case::kwargs_id(None, Some(text("kw-1")), Some("kw-1"))]
#[case::empty_kwargs_id(None, Some(text("")), Some(""))]
#[case::absent(None, None, None)]
#[case::non_text(None, Some(number(3)), None)]
fn resolve_call_id_reads_the_logging_object_then_the_request(
    #[case] logging_call_id: Option<&str>,
    #[case] field: Option<Json>,
    #[case] expected: Option<&str>,
) {
    let body = body_with(field);
    assert_eq!(resolve_call_id(logging_call_id, &body), expected);
}

#[tokio::test]
async fn the_success_log_names_the_call_id_counts_and_keys() {
    let (captured, _guard) = capture();
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(compressed_output(content(&[("a", "1"), ("b", "2")])))],
    );
    let mut body = request();
    body.insert("litellm_call_id".to_owned(), text("call-1"));
    applied(&harness, &mut body).await;
    let events = captured.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.level, Level::DEBUG);
    assert_eq!(event.kind, "compressed");
    assert_eq!(event.call_id.as_deref(), Some("call-1"));
    assert_eq!(event.original.as_deref(), Some("100"));
    assert_eq!(event.compressed.as_deref(), Some("40"));
    assert_eq!(event.cached_keys, Some(2));
}

#[tokio::test]
async fn the_skip_log_carries_the_reason() {
    let (captured, _guard) = capture();
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            50,
        ))],
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    let events = captured.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level, Level::DEBUG);
    assert_eq!(events[0].kind, "skipped");
    assert_eq!(events[0].reason.as_deref(), Some("below_trigger"));
    assert_eq!(events[0].original.as_deref(), Some("50"));
}

#[tokio::test]
async fn a_skip_without_a_reason_logs_nothing() {
    let (captured, _guard) = capture();
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(output(
            Vec::new(),
            Vec::new(),
            number(10),
            number(10),
            IndexMap::new(),
        ))],
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    assert!(captured.events().is_empty());
}

#[tokio::test]
async fn an_empty_cache_leaves_no_trace_on_the_request() {
    let harness = harness(
        Settings::default(),
        fixed_clock(NOW),
        vec![Ok(skipped_output(
            CompressionSkippedReason::BelowTrigger,
            50,
        ))],
    );
    let mut body = request();
    applied(&harness, &mut body).await;
    assert_eq!(body, request());
}
