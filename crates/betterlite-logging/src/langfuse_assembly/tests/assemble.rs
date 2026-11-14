#![cfg(test)]

use betterlite_models::decisions::Json;

use super::support::{
    Fixture, absent, attribute, epoch_plus, fixture, hashed, json, payload, payload_with, run,
    seconds, text, trace_id, value,
};

const CALL_ID: &str = "0102030405060708090a0b0c0d0e0f10";
const GUARDRAILS: &str = r#"[{
    "guardrail_name": "pii",
    "guardrail_mode": "pre_call",
    "start_time": 1704067200.0,
    "end_time": 1704067201.0,
    "guardrail_request": {"messages": []},
    "guardrail_response": {"ok": true}
}]"#;

fn new_trace_case() -> Fixture {
    let mut case = fixture();
    case.inputs.payload = Some(payload(r#"{"user_api_key_end_user_id": "end-user-1"}"#));
    case.inputs
        .set_metadata("session_id", Json::Text("session-x".to_owned()));
    case.inputs.input = Some(value(r#"[{"role": "user", "content": "hi"}]"#));
    case.inputs.output = Some(value(r#"{"role": "assistant", "content": "hello"}"#));
    case.inputs.litellm_call_id = Some(CALL_ID);
    case.inputs.start_time = Some(seconds(1_704_067_200));
    case.inputs.end_time = Some(seconds(1_704_067_201));
    case
}

#[test]
fn assembles_a_new_trace_and_generation() {
    let case = new_trace_case();
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(outcome.trace_id, Some(hashed("trace-1")));
    assert_eq!(
        outcome.generation_id,
        Some(generation.span_context.span_id().to_string())
    );
    assert_eq!(generation.name.to_string(), "litellm-completion");
    assert_eq!(generation.span_context.trace_id(), hashed("trace-1"));
    assert_eq!(
        text(generation, "langfuse.trace.name").as_deref(),
        Some("litellm-completion")
    );
    assert_eq!(text(generation, "session.id").as_deref(), Some("session-x"));
    assert_eq!(text(generation, "user.id").as_deref(), Some("end-user-1"));
}

#[test]
fn a_new_trace_carries_the_input_output_tags_and_end_time() {
    let case = new_trace_case();
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(generation, "langfuse.observation.input"),
        case.inputs.input
    );
    assert_eq!(
        json(generation, "langfuse.observation.output"),
        case.inputs.output
    );
    assert_eq!(
        attribute(generation, "langfuse.trace.tags"),
        Some(super::support::string_array(&["team-a"]))
    );
    assert_eq!(generation.end_time, epoch_plus(1_704_067_201));
}

#[test]
fn the_trace_keeps_the_generation_input_only_when_they_differ() {
    let mut same = fixture();
    same.inputs.input = Some(Json::Text("hi".to_owned()));
    let same_outcome = run(same.assemble());
    let same_generation = same_outcome.generation().expect("a generation span");
    assert!(absent(same_generation, "langfuse.trace.input"));
    assert!(absent(same_generation, "langfuse.trace.output"));

    let mut differing = fixture();
    differing
        .inputs
        .set_metadata("trace_input", Json::Text("the trace input".to_owned()));
    let differing_outcome = run(differing.assemble());
    let differing_generation = differing_outcome.generation().expect("a generation span");
    assert_eq!(
        text(differing_generation, "langfuse.trace.input").as_deref(),
        Some("the trace input")
    );
}

#[test]
fn an_existing_trace_updates_only_the_requested_keys() {
    let mut case = fixture();
    case.inputs.update_trace_keys = true;
    case.inputs
        .set_metadata("existing_trace_id", Json::Text(CALL_ID.to_owned()));
    case.inputs
        .set_metadata("trace_name", Json::Text("never used".to_owned()));
    case.inputs
        .set_metadata("trace_metadata", value(r#"{"env": "prod"}"#));
    case.inputs
        .set_metadata("update_trace_keys", value(r#"["trace_metadata"]"#));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(outcome.trace_id, Some(trace_id(CALL_ID)));
    assert_eq!(generation.span_context.trace_id(), trace_id(CALL_ID));
    assert!(generation.parent_span_is_remote);
    assert!(absent(generation, "langfuse.trace.name"));
    assert!(absent(generation, "langfuse.trace.tags"));
    assert_eq!(
        text(generation, "langfuse.trace.metadata.env").as_deref(),
        Some("prod")
    );
}

#[test]
fn an_existing_trace_drops_keys_update_trace_keys_does_not_name() {
    let mut case = fixture();
    case.inputs
        .set_metadata("existing_trace_id", Json::Text(CALL_ID.to_owned()));
    case.inputs
        .set_metadata("trace_metadata", value(r#"{"env": "prod"}"#));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(generation, "langfuse.trace.metadata.env"));
}

#[test]
fn child_spans_hang_under_the_generation() {
    let mut case = fixture();
    case.inputs.payload = Some(payload_with(
        "{}",
        r#"{"cache_key": "hidden-1"}"#,
        GUARDRAILS,
    ));
    case.inputs.litellm_call_id = Some(CALL_ID);
    let outcome = run(case.assemble());
    assert_eq!(outcome.names(), vec!["guardrail", "litellm-completion"]);
    let generation = outcome.generation().expect("a generation span");
    let guardrail = outcome.by_name("guardrail").expect("a guardrail span");
    assert_eq!(guardrail.parent_span_id, generation.span_context.span_id());
    assert_eq!(
        text(guardrail, "langfuse.observation.metadata.guardrail_name").as_deref(),
        Some("pii")
    );
}

#[test]
fn a_malformed_usage_returns_nothing_and_logs_no_span() {
    let mut case = fixture();
    case.inputs.response = Some(value(r#"{"usage": {"prompt_tokens": "many"}}"#));
    let outcome = run(case.assemble());
    assert_eq!(outcome.trace_id, None);
    assert_eq!(outcome.generation_id, None);
    assert!(outcome.spans.is_empty());
}

#[test]
fn the_public_flag_reaches_the_generation() {
    let mut case = fixture();
    case.inputs.set_metadata("trace_public", Json::Bool(true));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        attribute(generation, "langfuse.trace.public"),
        Some(opentelemetry::Value::Bool(true))
    );
}
