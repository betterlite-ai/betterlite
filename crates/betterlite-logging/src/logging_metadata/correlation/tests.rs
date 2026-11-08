#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
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

#[test]
fn a_recorded_trace_id_is_found_by_its_call_and_service() {
    let mut cache = TraceIdCache::default();
    cache.record("call-1", TraceService::Langfuse, "trace-1".to_owned());
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse),
        Some("trace-1")
    );
    assert_eq!(
        cache.logged_trace_id("call-2", TraceService::Langfuse),
        None
    );
}

#[test]
fn a_later_record_replaces_the_trace_id_for_its_call() {
    let mut cache = TraceIdCache::default();
    cache.record("call-1", TraceService::Langfuse, "trace-1".to_owned());
    cache.record("call-1", TraceService::Langfuse, "trace-2".to_owned());
    assert_eq!(
        cache.logged_trace_id("call-1", TraceService::Langfuse),
        Some("trace-2")
    );
}

#[test]
fn empty_strings_are_values_the_caller_chose() {
    let record = params("{\"litellm_trace_id\": \"\", \"metadata\": {\"trace_id\": \"m-trace\"}}");
    assert_eq!(logging_payload_trace_id(&record, "generated", true), "");
}

#[test]
fn a_non_text_metadata_id_is_ignored() {
    let record = params("{\"metadata\": {\"trace_id\": 7, \"session_id\": 8}}");
    assert_eq!(
        logging_payload_trace_id(&record, "generated", true),
        "generated"
    );
    assert_eq!(
        logging_payload_session_id(&record, "generated", true),
        "generated"
    );
}

#[test]
fn metadata_json_is_built_from_the_wire_shape() {
    let metadata: IndexMap<String, Json> =
        serde_json::from_str(r#"{"trace_id": "t"}"#).expect("metadata decodes");
    assert_eq!(metadata.get("trace_id"), Some(&Json::Text("t".to_owned())));
}
