#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::agent_requests::prepare::Headers;
use crate::agent_runs::dispatch::{Route, dispatch};
use crate::agent_runs::flow::CompletionCall;
use crate::agent_runs::transport::{AgentRunMethod, AgentRunRequest};

use super::support::{
    CallFixture, Outcome, RecordingResolver, Run, SECRET, Step, execute, finish, messages,
    prepared, secret_headers, status_step, step,
};

const FIRST_KEY: &str = "fixture-first-key-4c1d";
const SECOND_KEY: &str = "fixture-second-key-9a7b";
const FIRST_BASE: &str = "https://first.example/api/projects/p";
const SECOND_BASE: &str = "https://second.example/api/projects/p";

fn authorization(headers: &Headers) -> String {
    headers
        .get("Authorization")
        .map(|value| value.expose_secret().to_owned())
        .expect("an Authorization header")
}

fn four_steps() -> Vec<Step> {
    vec![
        step(201, r#"{"id": "thread-1"}"#),
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        step(
            200,
            r#"{"data": [{"role": "assistant", "content": [{"type": "text", "text": {"value": "ok"}}]}]}"#,
        ),
    ]
}

#[test]
fn agent_runs_credential_identity_isolation() {
    let resolver = RecordingResolver::new(Some("resolver-token"));
    let mut first = CallFixture::new(FIRST_BASE);
    first.headers = secret_headers(&[("X-Trace", "trace-1")]);
    first.api_key = Some(SecretString::from(FIRST_KEY));
    let mut second = CallFixture::new(SECOND_BASE);
    second.headers = secret_headers(&[("X-Trace", "trace-1")]);
    second.api_key = Some(SecretString::from(SECOND_KEY));
    second.acompletion = true;
    let Route::Completion(first_call) = dispatch(first.call(), &resolver).expect("a route") else {
        panic!("a sync call is not a stream");
    };
    let Route::Completion(second_call) = dispatch(second.call(), &resolver).expect("a route")
    else {
        panic!("a non-streaming async call is not a stream");
    };
    assert_isolation(&first_call, &second_call, &first.headers, &resolver);
    let first_outcome = finish(&first_call, four_steps());
    let second_outcome = finish(&second_call, four_steps());
    assert_isolated_requests(&first_outcome, &second_outcome);
}

fn assert_isolation(
    first: &CompletionCall<'_>,
    second: &CompletionCall<'_>,
    caller_headers: &Headers,
    resolver: &RecordingResolver,
) {
    assert!(
        resolver.params().is_empty(),
        "an explicit key skips resolution"
    );
    assert_eq!(
        authorization(&first.prepared.headers),
        format!("Bearer {FIRST_KEY}")
    );
    assert_eq!(
        authorization(&second.prepared.headers),
        format!("Bearer {SECOND_KEY}")
    );
    assert_eq!(first.prepared.headers.len(), 3);
    assert_eq!(
        caller_headers.len(),
        1,
        "preparation leaves the caller's headers alone"
    );
    assert_eq!(first.prepared.agent_id, second.prepared.agent_id);
    assert_ne!(first.prepared.api_base, second.prepared.api_base);
}

fn assert_isolated_requests(first: &Outcome, second: &Outcome) {
    let first_state = first.transport.state();
    assert_eq!(first_state.requests().len(), 4);
    let first_request = first_state.requests()[0].clone();
    drop(first_state);
    let second_state = second.transport.state();
    assert_eq!(second_state.requests().len(), 4);
    let second_request = second_state.requests()[0].clone();
    drop(second_state);
    assert_eq!(
        first_request
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some(format!("Bearer {FIRST_KEY}").as_str())
    );
    assert_eq!(
        second_request
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some(format!("Bearer {SECOND_KEY}").as_str())
    );
    assert!(first_request.url.starts_with(FIRST_BASE));
    assert!(second_request.url.starts_with(SECOND_BASE));
    assert!(!first_request.url.contains("second.example"));
    assert!(!second_request.url.contains("first.example"));
}

#[test]
fn agent_runs_secret_safe_failure() {
    let held = prepared(&Object::new(), Some(&SecretString::from(SECRET)));
    assert!(!format!("{held:?}").contains(SECRET));
    assert_eq!(authorization(&held.headers), format!("Bearer {SECRET}"));
    let request = AgentRunRequest {
        method: AgentRunMethod::Post,
        url: String::from("https://base.example/api/threads?api-version=2025-05-01"),
        headers: held.headers,
        body: Some(b"{}".to_vec()),
    };
    let shown = format!("{request:?}");
    assert!(!shown.contains(SECRET));
    assert!(shown.contains("header_names"));
    assert!(shown.contains("Authorization"));
    assert!(shown.contains("Content-Type"));
    assert!(shown.contains("body_length: Some(2)"));
    secret_safe_failure_message();
}

fn secret_safe_failure_message() {
    let mut case = Run::sync(vec![step(500, "denied")]);
    case.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let outcome = execute(&case);
    let error = outcome.result.expect_err("a failure");
    assert_eq!(error.to_string(), "Failed to create thread: denied");
    assert!(!error.to_string().contains(SECRET));
    assert!(!format!("{error:?}").contains(SECRET));
    let state = outcome.transport.state();
    assert_eq!(
        state.requests().len(),
        1,
        "no later request after the failure"
    );
    assert_eq!(
        state.requests()[0]
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some(format!("Bearer {SECRET}").as_str())
    );
    assert_eq!(state.remaining(), 0);
}
