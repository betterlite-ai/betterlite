use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::fixtures::{call_kwargs, ids, object, payload, sources};
use crate::fields::payload::StandardLoggingPayload;
use crate::newrelic_extraction::ids::{SystemUuids, UuidSource};
use crate::newrelic_extraction::trace_context::resolve_trace_id;

fn trace_of(
    headers: Option<&IndexMap<String, Json>>,
    standard: Option<&StandardLoggingPayload>,
    source: &dyn UuidSource,
) -> String {
    let mut kwargs = call_kwargs();
    kwargs.headers = headers;
    let response = object("{}");
    let event_sources = sources(&kwargs, &response, standard);
    resolve_trace_id(&event_sources, source)
}

#[rstest]
#[case::lowercase_name(
    r#"{"traceparent":"00-4bf92f3577b34da6a3ce929d0e0e4736-b7ad6b7169203331-00"}"#,
    "4bf92f3577b34da6a3ce929d0e0e4736"
)]
#[case::mixed_case_name(
    r#"{"TraceParent":"00-11111111111111111111111111111111-2222222222222222-00"}"#,
    "11111111111111111111111111111111"
)]
#[case::upper_case_name(
    r#"{"TRACEPARENT":"00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"}"#,
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
)]
#[case::other_header_first(
    r#"{"x-request-id":"abc","traceparent":"00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-00"}"#,
    "cccccccccccccccccccccccccccccccc"
)]
#[case::unvalidated_trace(r#"{"traceparent":"00-nothex-span-00"}"#, "nothex")]
fn newrelic_extraction_tests_behavior_trace_context_reads_traceparent(
    #[case] headers: &str,
    #[case] expected: &str,
) {
    let fields = object(headers);
    assert_eq!(trace_of(Some(&fields), None, &ids(7)), expected);
}

#[rstest]
#[case::three_parts(r#"{"traceparent":"00-trace-span"}"#)]
#[case::five_parts(r#"{"traceparent":"00-trace-span-00-extra"}"#)]
#[case::empty_trace(r#"{"traceparent":"00--span-00"}"#)]
#[case::number_value(r#"{"traceparent":7}"#)]
#[case::no_header(r#"{"x-request-id":"abc"}"#)]
fn newrelic_extraction_tests_behavior_trace_context_falls_back_to_payload(#[case] headers: &str) {
    let standard = payload();
    let fields = object(headers);
    assert_eq!(trace_of(Some(&fields), Some(&standard), &ids(7)), "trace-1");
}

#[test]
fn newrelic_extraction_tests_behavior_trace_context_prefers_the_header() {
    let standard = payload();
    let fields =
        object(r#"{"traceparent":"00-11111111111111111111111111111111-2222222222222222-00"}"#);
    assert_eq!(
        trace_of(Some(&fields), Some(&standard), &ids(7)),
        "11111111111111111111111111111111"
    );
}

#[test]
fn newrelic_extraction_tests_behavior_trace_context_generates_a_hex_id() {
    assert_eq!(
        trace_of(None, None, &ids(7)),
        "00000000000000000000000000000007"
    );
}

#[test]
fn newrelic_extraction_tests_behavior_trace_context_generates_32_hex_digits() {
    let trace = trace_of(None, None, &SystemUuids);
    assert_eq!(trace.len(), 32);
    assert!(!trace.contains('-'));
    assert!(trace.chars().all(|character| character.is_ascii_hexdigit()));
}

#[test]
fn newrelic_extraction_tests_behavior_trace_context_generates_when_the_payload_trace_is_empty() {
    let mut standard = payload();
    standard.trace_id = String::new();
    assert_eq!(
        trace_of(None, Some(&standard), &ids(7)),
        "00000000000000000000000000000007"
    );
}

#[test]
fn newrelic_extraction_tests_behavior_trace_context_never_emits_the_parent_span() {
    let fields =
        object(r#"{"traceparent":"00-4bf92f3577b34da6a3ce929d0e0e4736-b7ad6b7169203331-00"}"#);
    let trace = trace_of(Some(&fields), None, &ids(7));
    assert_eq!(trace, "4bf92f3577b34da6a3ce929d0e0e4736");
    assert_ne!(trace, "b7ad6b7169203331");
}
