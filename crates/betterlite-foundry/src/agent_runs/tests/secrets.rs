#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::agent_requests::prepare::Headers;
use crate::agent_results::reply::AgentReply;
use crate::agent_runs::dispatch::{DispatchCall, Route, dispatch};
use crate::agent_runs::transport::{AgentRunMethod, AgentRunRequest};

use super::support::{
    MODEL, RecordingResolver, Run, SECRET, Step, execute, finish, messages, prepared, status_step,
    step,
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
    let empty = Object::new();
    let caller_headers: Headers =
        BTreeMap::from([(String::from("X-Trace"), SecretString::from("trace-1"))]);
    let environment = BTreeMap::new();
    let resolver = RecordingResolver::new(Some("resolver-token"));
    let first_key = SecretString::from(FIRST_KEY);
    let second_key = SecretString::from(SECOND_KEY);

    let first = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &empty,
        litellm_params: &empty,
        headers: &caller_headers,
        environment: &environment,
        api_base: Some(FIRST_BASE),
        api_key: Some(&first_key),
        reply: AgentReply::default(),
        acompletion: false,
        stream: false,
    };
    let Route::Completion(first_call) = dispatch(first, &resolver).expect("a route") else {
        panic!("a sync call is not a stream");
    };
    let second = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &empty,
        litellm_params: &empty,
        headers: &caller_headers,
        environment: &environment,
        api_base: Some(SECOND_BASE),
        api_key: Some(&second_key),
        reply: AgentReply::default(),
        acompletion: true,
        stream: false,
    };
    let Route::Completion(second_call) = dispatch(second, &resolver).expect("a route") else {
        panic!("a non-streaming async call is not a stream");
    };
    assert!(
        resolver.params().is_empty(),
        "an explicit key skips resolution"
    );

    assert_eq!(
        authorization(&first_call.prepared.headers),
        format!("Bearer {FIRST_KEY}")
    );
    assert_eq!(
        authorization(&second_call.prepared.headers),
        format!("Bearer {SECOND_KEY}")
    );
    assert_eq!(first_call.prepared.headers.len(), 3);
    assert_eq!(
        caller_headers.len(),
        1,
        "preparation leaves the caller's headers alone"
    );
    assert_eq!(first_call.prepared.agent_id, second_call.prepared.agent_id);
    assert_ne!(first_call.prepared.api_base, second_call.prepared.api_base);

    let first_outcome = finish(&first_call, four_steps());
    let second_outcome = finish(&second_call, four_steps());
    assert_eq!(first_outcome.transport.state().requests().len(), 4);
    assert_eq!(second_outcome.transport.state().requests().len(), 4);
    let first_headers = first_outcome.transport.state().requests()[0]
        .headers
        .clone();
    let second_headers = second_outcome.transport.state().requests()[0]
        .headers
        .clone();
    assert_eq!(
        first_headers.get("Authorization").map(String::as_str),
        Some(format!("Bearer {FIRST_KEY}").as_str())
    );
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some(format!("Bearer {SECOND_KEY}").as_str())
    );
    let first_url = first_outcome.transport.state().requests()[0].url.clone();
    let second_url = second_outcome.transport.state().requests()[0].url.clone();
    assert!(first_url.starts_with(FIRST_BASE));
    assert!(second_url.starts_with(SECOND_BASE));
    assert!(!first_url.contains("second.example"));
    assert!(!second_url.contains("first.example"));

    let mut widened = first_call.prepared.headers.clone();
    widened.insert(
        String::from("X-Session"),
        SecretString::from("first-session"),
    );
    assert!(!second_call.prepared.headers.contains_key("X-Session"));
    assert_eq!(
        authorization(&second_call.prepared.headers),
        format!("Bearer {SECOND_KEY}")
    );
}

#[test]
fn agent_runs_secret_safe_failure() {
    let held = prepared(&Object::new(), Some(&SecretString::from(SECRET)));
    assert!(!format!("{held:?}").contains(SECRET));
    assert_eq!(authorization(&held.headers), format!("Bearer {SECRET}"));

    let request = AgentRunRequest {
        method: AgentRunMethod::Post,
        url: String::from("https://base.example/api/threads?api-version=2025-05-01"),
        headers: held.headers.clone(),
        body: Some(b"{}".to_vec()),
    };
    let shown = format!("{request:?}");
    assert!(!shown.contains(SECRET));
    assert!(shown.contains("header_names"));
    assert!(shown.contains("Authorization"));
    assert!(shown.contains("Content-Type"));
    assert!(shown.contains("body_length: Some(2)"));

    let mut case = Run::sync(vec![step(500, "denied")]);
    case.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let outcome = execute(&case);
    let error = outcome.result.err().expect("a failure");
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
