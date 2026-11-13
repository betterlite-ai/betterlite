#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{Fixture, hashed, payload, run, text, trace_id, value};

const CALL_ID: &str = "0102030405060708090a0b0c0d0e0f10";
const SESSION: &str = "session-abc-123";

fn with_headers(headers: &str) -> Json {
    value(&format!(
        r#"{{"proxy_server_request": {{"method": "POST", "url": "http://proxy/v1/chat", "headers": {headers}}}}}"#
    ))
}

fn session_fixture() -> Fixture {
    let mut fixture = Fixture::default();
    fixture.inputs.payload = Some(payload("{}"));
    fixture
        .inputs
        .set_metadata("session_id", Json::Text(SESSION.to_owned()));
    fixture
        .inputs
        .set_metadata("trace_id", Json::Text(SESSION.to_owned()));
    fixture
}

#[test]
fn metadata_trace_id_wins_over_the_payload_and_the_call_id() {
    let mut fixture = Fixture::default();
    fixture.inputs.payload = Some(payload("{}"));
    fixture
        .inputs
        .set_metadata("trace_id", Json::Text(CALL_ID.to_owned()));
    fixture.inputs.litellm_call_id = Some("call-1");
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(generation.span_context.trace_id(), trace_id(CALL_ID));
}

#[test]
fn the_payload_trace_id_is_the_first_fallback() {
    let mut fixture = Fixture::default();
    fixture.inputs.payload = Some(payload("{}"));
    fixture.inputs.litellm_call_id = Some("call-1");
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("trace-1")));
}

#[test]
fn the_call_id_is_the_last_fallback() {
    let mut fixture = Fixture::default();
    fixture.inputs.litellm_call_id = Some("call-1");
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("call-1")));
}

#[test]
fn the_litellm_trace_id_beats_the_call_id() {
    let mut fixture = Fixture::default();
    fixture.inputs.litellm_trace_id = Some("trace-9");
    fixture.inputs.litellm_call_id = Some("call-1");
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("trace-9")));
}

#[test]
fn an_existing_trace_id_wins_over_every_other_source() {
    let mut fixture = Fixture::default();
    fixture.inputs.payload = Some(payload("{}"));
    fixture.inputs.litellm_call_id = Some("call-1");
    fixture
        .inputs
        .set_metadata("trace_id", Json::Text("another-trace".to_owned()));
    fixture
        .inputs
        .set_metadata("existing_trace_id", Json::Text(CALL_ID.to_owned()));
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
}

#[test]
fn a_session_header_gives_each_call_its_own_trace() {
    let mut fixture = session_fixture();
    fixture.inputs.litellm_call_id = Some(CALL_ID);
    fixture.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(generation.span_context.trace_id(), trace_id(CALL_ID));
    assert_eq!(
        text(generation, "langfuse.trace.session_id").as_deref(),
        Some(SESSION)
    );
}

#[test]
fn a_session_header_without_a_call_id_keeps_the_session_trace() {
    let mut fixture = session_fixture();
    fixture.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(hashed(SESSION)));
}

#[test]
fn a_custom_session_id_header_continues_the_trace() {
    let mut fixture = session_fixture();
    fixture.inputs.litellm_call_id = Some(CALL_ID);
    fixture.inputs.litellm_params = with_headers(r#"{"x-session-id": "session-abc-123"}"#);
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
}

#[test]
fn a_langfuse_trace_header_disables_session_detection() {
    let mut fixture = session_fixture();
    fixture.inputs.litellm_call_id = Some(CALL_ID);
    fixture.inputs.litellm_params = with_headers(
        r#"{"x-litellm-session-id": "session-abc-123", "langfuse_trace_id": "elsewhere"}"#,
    );
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(hashed(SESSION)));
}

#[test]
fn an_existing_trace_never_goes_through_session_detection() {
    let mut fixture = session_fixture();
    fixture.inputs.litellm_call_id = Some(CALL_ID);
    fixture.inputs.litellm_params = with_headers(r#"{"x-litellm-session-id": "session-abc-123"}"#);
    fixture
        .inputs
        .set_metadata("existing_trace_id", Json::Text("existing-trace".to_owned()));
    let outcome = run(fixture.assemble());
    assert_eq!(outcome.trace_id, Some(hashed("existing-trace")));
}
