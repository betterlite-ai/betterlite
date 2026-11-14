#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{Fixture, capture_logs, fixture, hashed, payload, run, text, trace_id, value};

const CALL_ID: &str = "0102030405060708090a0b0c0d0e0f10";
const SESSION: &str = "session-abc-123";

fn with_headers(headers: &str) -> Json {
    value(&format!(
        r#"{{"proxy_server_request": {{"method": "POST", "url": "http://proxy/v1/chat", "headers": {headers}}}}}"#
    ))
}

fn session_fixture() -> Fixture {
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs
        .set_metadata("session_id", Json::Text(SESSION.to_owned()));
    case.inputs
        .set_metadata("trace_id", Json::Text(SESSION.to_owned()));
    case
}

#[test]
fn metadata_trace_id_wins_over_the_payload_and_the_call_id() {
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs
        .set_metadata("trace_id", Json::Text(CALL_ID.to_owned()));
    case.inputs.litellm_call_id = Some("call-1");
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(generation.span_context.trace_id(), trace_id(CALL_ID));
}

#[test]
fn the_payload_trace_id_is_the_first_fallback() {
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs.litellm_call_id = Some("call-1");
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("trace-1")));
}

#[test]
fn the_call_id_is_the_last_fallback() {
    let mut case = fixture();
    case.inputs.litellm_call_id = Some("call-1");
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("call-1")));
}

#[test]
fn the_litellm_trace_id_beats_the_call_id() {
    let mut case = fixture();
    case.inputs.litellm_trace_id = Some("trace-9");
    case.inputs.litellm_call_id = Some("call-1");
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("trace-9")));
}

#[test]
fn an_existing_trace_id_wins_over_every_other_source() {
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs.litellm_call_id = Some("call-1");
    case.inputs
        .set_metadata("trace_id", Json::Text("another-trace".to_owned()));
    case.inputs
        .set_metadata("existing_trace_id", Json::Text(CALL_ID.to_owned()));
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
}

#[test]
fn a_session_header_gives_each_call_its_own_trace() {
    let mut case = session_fixture();
    case.inputs.litellm_call_id = Some(CALL_ID);
    case.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(generation.span_context.trace_id(), trace_id(CALL_ID));
    assert_eq!(text(generation, "session.id").as_deref(), Some(SESSION));
}

#[test]
fn session_reuse_logs_the_call_trace_that_replaces_the_session_trace() {
    let (captured, _guard) = capture_logs();
    let mut case = session_fixture();
    case.inputs.litellm_call_id = Some(CALL_ID);
    case.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    let _outcome = run(case.assemble());
    assert!(captured.text().contains("came from a session header"));
}

#[test]
fn a_plain_trace_id_logs_no_session_reuse() {
    let (captured, _guard) = capture_logs();
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs.litellm_call_id = Some(CALL_ID);
    let _outcome = run(case.assemble());
    assert!(!captured.text().contains("came from a session header"));
}

#[test]
fn a_session_header_without_a_call_id_keeps_the_session_trace() {
    let mut case = session_fixture();
    case.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(hashed(SESSION)));
}

#[test]
fn a_custom_session_id_header_continues_the_trace() {
    let mut case = session_fixture();
    case.inputs.litellm_call_id = Some(CALL_ID);
    case.inputs.litellm_params = with_headers(r#"{"x-session-id": "session-abc-123"}"#);
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
}

#[test]
fn a_langfuse_trace_header_disables_session_detection() {
    let mut case = session_fixture();
    case.inputs.litellm_call_id = Some(CALL_ID);
    case.inputs.litellm_params = with_headers(
        r#"{"x-litellm-session-id": "session-abc-123", "langfuse_trace_id": "elsewhere"}"#,
    );
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(hashed(SESSION)));
}

#[test]
fn an_existing_trace_never_goes_through_session_detection() {
    let mut case = session_fixture();
    case.inputs.litellm_call_id = Some(CALL_ID);
    case.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    case.inputs
        .set_metadata("existing_trace_id", Json::Text("existing-trace".to_owned()));
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("existing-trace")));
}
