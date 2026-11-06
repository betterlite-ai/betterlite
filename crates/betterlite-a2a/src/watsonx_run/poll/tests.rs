#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{MAX_POLL_ATTEMPTS, PollError, poll_run, run_id};
use crate::watsonx_run::event::RunEvent;
use crate::watsonx_run::run_api::ApiError;
use crate::watsonx_run::tests::{Fail, Reply, Script, limits};

const BASE: &str = "https://cp4d.example.com/orchestrate/cpd/instances/inst-1";
const RUN: &str =
    "https://cp4d.example.com/orchestrate/cpd/instances/inst-1/v1/orchestrate/runs/run-1";
const TIMEOUT: Duration = Duration::from_secs(90);

fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([("Authorization".to_owned(), "Bearer tok-1".to_owned())])
}

fn event(body: &str) -> RunEvent {
    serde_json::from_str(body).unwrap()
}

fn started() -> RunEvent {
    event(r#"{"run_id": "run-1", "status": "running"}"#)
}

#[tokio::test]
async fn polling_waits_two_seconds_before_every_get_on_the_run_id_endpoint() {
    let script = Script::new(vec![
        Reply::json(r#"{"status": "running"}"#),
        Reply::json(r#"{"status": "completed"}"#),
    ]);
    let headers = headers();
    let polled = poll_run(&&script, BASE, started(), &headers, limits(TIMEOUT))
        .await
        .unwrap();
    assert_eq!(polled.status.as_deref(), Some("completed"));
    assert_eq!(*script.pauses.lock().unwrap(), [Duration::from_secs(2); 2]);
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].method, "GET");
    assert_eq!(recorded[0].url, RUN);
    assert_eq!(recorded[0].headers, headers);
    assert_eq!(recorded[0].timeout, TIMEOUT);
    assert_eq!(recorded[0].body, AgentJson::Null);
}

#[rstest]
#[case::completed("completed")]
#[case::succeeded("succeeded")]
#[tokio::test]
async fn polling_returns_at_the_first_successful_terminal_state(#[case] status: &str) {
    let answer = format!(r#"{{"run_id": "run-1", "status": "{status}"}}"#);
    let script = Script::new(vec![Reply::json(&answer)]);
    let polled = poll_run(&&script, BASE, started(), &headers(), limits(TIMEOUT))
        .await
        .unwrap();
    assert_eq!(polled.status.as_deref(), Some(status));
    assert_eq!(script.recorded().len(), 1);
}

#[tokio::test]
async fn polling_times_out_after_ninety_attempts() {
    let script = Script::new(vec![Reply::json(r#"{"status": "running"}"#).repeating()]);
    let error = poll_run(&&script, BASE, started(), &headers(), limits(TIMEOUT))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the watsonx Orchestrate run 'run-1' did not reach a terminal state after 180s"
    );
    let attempts = usize::try_from(MAX_POLL_ATTEMPTS).unwrap();
    assert!(matches!(error, PollError::PollTimeout { seconds: 180, .. }));
    assert_eq!(script.pause_count(), attempts);
    assert_eq!(script.recorded().len(), attempts);
}

#[rstest]
#[case::redirect(302)]
#[case::not_found(404)]
#[case::server_error(500)]
#[tokio::test]
async fn polling_fails_on_a_status_that_is_not_a_success(#[case] status: u16) {
    let script = Script::new(vec![Reply::json("{}").with_status(status)]);
    let error = poll_run(&&script, BASE, started(), &headers(), limits(TIMEOUT))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PollError::Api(ApiError::Status { status: got }) if got == status
    ));
}

#[tokio::test]
async fn polling_surfaces_a_transport_failure() {
    let script = Script::new(vec![Reply::failing(Fail::Transport)]);
    let error = poll_run(&&script, BASE, started(), &headers(), limits(TIMEOUT))
        .await
        .unwrap_err();
    assert!(matches!(error, PollError::Api(ApiError::Transport(_))));
}

#[tokio::test]
async fn polling_does_not_poll_a_run_that_is_already_terminal() {
    let script = Script::new(Vec::new());
    let run = poll_run(
        &&script,
        BASE,
        event(r#"{"run_id": "run-1", "status": "completed"}"#),
        &headers(),
        limits(TIMEOUT),
    )
    .await
    .unwrap();
    assert_eq!(run.status.as_deref(), Some("completed"));
    assert_eq!(script.recorded().len(), 0);
}

#[tokio::test]
async fn polling_polls_a_run_that_is_not_terminal_and_returns_the_terminal_one() {
    let script = Script::new(vec![Reply::json(
        r#"{"run_id": "run-1", "status": "succeeded"}"#,
    )]);
    let run = poll_run(&&script, BASE, started(), &headers(), limits(TIMEOUT))
        .await
        .unwrap();
    assert_eq!(run.status.as_deref(), Some("succeeded"));
    assert_eq!(script.recorded()[0].url, RUN);
}

#[tokio::test]
async fn polling_falls_back_to_the_id_when_the_run_id_is_empty() {
    let script = Script::new(vec![Reply::json(r#"{"status": "completed"}"#)]);
    let run = poll_run(
        &&script,
        BASE,
        event(r#"{"run_id": "", "id": "run-1", "status": "running"}"#),
        &headers(),
        limits(TIMEOUT),
    )
    .await
    .unwrap();
    assert_eq!(run.status.as_deref(), Some("completed"));
    assert_eq!(script.recorded()[0].url, RUN);
}

#[tokio::test]
async fn polling_falls_back_to_the_id_when_the_run_id_is_absent() {
    let script = Script::new(vec![Reply::json(r#"{"status": "completed"}"#)]);
    let run = poll_run(
        &&script,
        BASE,
        event(r#"{"id": "run-1", "status": "running"}"#),
        &headers(),
        limits(TIMEOUT),
    )
    .await
    .unwrap();
    assert_eq!(run.status.as_deref(), Some("completed"));
    assert_eq!(script.recorded()[0].url, RUN);
}

#[test]
fn polling_reads_the_run_id_before_the_id() {
    assert_eq!(
        run_id(&event(r#"{"run_id": "run-1", "id": "run-2"}"#)),
        Some("run-1")
    );
    assert_eq!(
        run_id(&event(r#"{"run_id": "", "id": "run-2"}"#)),
        Some("run-2")
    );
    assert_eq!(run_id(&event(r#"{"status": "running"}"#)), None);
}

#[tokio::test]
async fn polling_requires_a_run_id_for_a_run_that_is_not_terminal() {
    let script = Script::new(Vec::new());
    let error = poll_run(
        &&script,
        BASE,
        event(r#"{"status": "running"}"#),
        &headers(),
        limits(TIMEOUT),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, PollError::NoRunId));
    assert_eq!(script.recorded().len(), 0);
}

#[rstest]
#[case::failed("failed")]
#[case::error("error")]
#[case::cancelled("cancelled")]
#[tokio::test]
async fn polling_fails_a_terminal_run_that_is_not_a_success(#[case] status: &str) {
    let script = Script::new(Vec::new());
    let answer = format!(r#"{{"run_id": "run-1", "status": "{status}"}}"#);
    let error = poll_run(&&script, BASE, event(&answer), &headers(), limits(TIMEOUT))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the watsonx Orchestrate run ended with a non-success status '{status}'")
    );
    assert_eq!(script.recorded().len(), 0);
}

#[rstest]
#[case::failed("failed")]
#[case::error("error")]
#[case::cancelled("cancelled")]
#[tokio::test]
async fn polling_stops_at_a_polled_run_that_ended_without_success(#[case] status: &str) {
    let answer = format!(r#"{{"status": "{status}"}}"#);
    let script = Script::new(vec![Reply::json(&answer)]);
    let error = poll_run(&&script, BASE, started(), &headers(), limits(TIMEOUT))
        .await
        .unwrap_err();
    assert!(matches!(error, PollError::NonSuccess { .. }));
    assert_eq!(script.recorded().len(), 1);
}

#[tokio::test]
async fn polling_keeps_the_timeout_it_was_given_for_every_get() {
    let script = Script::new(vec![
        Reply::json(r#"{"status": "running"}"#),
        Reply::json(r#"{"status": "completed"}"#),
    ]);
    let timeout = Duration::from_secs(120);
    poll_run(&&script, BASE, started(), &headers(), limits(timeout))
        .await
        .unwrap();
    let timeouts: Vec<_> = script
        .recorded()
        .iter()
        .map(|request| request.timeout)
        .collect();
    assert_eq!(timeouts, [timeout; 2]);
}
