#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use rstest::rstest;
use thiserror::Error;
use uuid::Uuid;

use super::error::SubmitError;
use super::{
    AgentCall, AgentRequest, AgentResponse, AgentTransport, DEFAULT_TIMEOUT, send_and_poll_raw,
};
use crate::completion_bridge::EventSource;

#[derive(Debug, Error)]
#[error("the scripted transport has no response left")]
pub(crate) struct ScriptExhausted;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) url: String,
    pub(crate) body: AgentJson,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) timeout: Duration,
}

#[derive(Default)]
pub(crate) struct ScriptedTransport {
    responses: Mutex<VecDeque<AgentResponse>>,
    pub(crate) requests: Mutex<Vec<Recorded>>,
    pub(crate) pauses: Mutex<Vec<Duration>>,
}

impl ScriptedTransport {
    pub(crate) fn answering(bodies: &[&str]) -> Self {
        let responses = bodies.iter().map(|body| ok(body)).collect();
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
            pauses: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub(crate) fn pause_count(&self) -> usize {
        self.pauses.lock().unwrap().len()
    }
}

impl AgentTransport for ScriptedTransport {
    type Error = ScriptExhausted;

    fn post(
        &self,
        request: &AgentRequest<'_>,
    ) -> impl Future<Output = Result<AgentResponse, ScriptExhausted>> + Send {
        self.requests.lock().unwrap().push(Recorded {
            url: request.url.to_owned(),
            body: serde_json::to_value(request.body).unwrap(),
            headers: request.headers.clone(),
            timeout: request.timeout,
        });
        ready(
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or(ScriptExhausted),
        )
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses.lock().unwrap().push(duration);
        ready(())
    }
}

pub(crate) fn json(text: &str) -> AgentJson {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn ok(body: &str) -> AgentResponse {
    AgentResponse {
        status: 200,
        body: json(body),
    }
}

const SUBMITTED: &str = r#"{"result": {"id": "task-1", "status": {"state": "submitted"}}}"#;
const WORKING: &str = r#"{"result": {"id": "task-1", "status": {"state": "working"}}}"#;
const COMPLETED: &str = r#"{"result": {"id": "task-1", "status": {"state": "completed"}, "artifacts": [{"parts": [{"kind": "text", "text": "done"}]}]}}"#;

async fn send(
    transport: &ScriptedTransport,
    api_base: &str,
    headers: &BTreeMap<String, String>,
) -> Result<AgentJson, SubmitError<ScriptExhausted>> {
    let params = json(r#"{"message": {"role": "user", "parts": [], "extra": null}}"#);
    send_and_poll_raw(
        transport,
        &AgentCall {
            api_base,
            request_id: "req-1",
            params: &params,
            timeout: DEFAULT_TIMEOUT,
            headers,
        },
    )
    .await
}

#[tokio::test]
async fn pydantic_ai_submit_posts_message_send_to_the_trimmed_endpoint() {
    let transport = ScriptedTransport::answering(&[COMPLETED]);
    let raw = send(
        &transport,
        "http://agent.test:9999/a2a///",
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(raw, json(COMPLETED));
    assert_eq!(
        transport.recorded(),
        [Recorded {
            url: "http://agent.test:9999/a2a".to_owned(),
            body: json(
                r#"{"jsonrpc": "2.0", "id": "req-1", "method": "message/send",
                    "params": {"message": {"kind": "message", "role": "user", "parts": []}}}"#
            ),
            headers: BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]),
            timeout: Duration::from_secs(60),
        }]
    );
    assert_eq!(transport.pause_count(), 0);
}

#[tokio::test]
async fn pydantic_ai_submit_keeps_the_query_string_when_trimming_slashes() {
    let transport = ScriptedTransport::answering(&[COMPLETED]);
    send(&transport, "http://agent.test/?key=1/", &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(transport.recorded()[0].url, "http://agent.test/?key=1");
}

#[tokio::test]
async fn pydantic_ai_submit_polls_until_completed_with_numbered_request_ids() {
    let transport = ScriptedTransport::answering(&[SUBMITTED, WORKING, WORKING, COMPLETED]);
    let raw = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(raw, json(COMPLETED));
    let recorded = transport.recorded();
    let polls: Vec<_> = recorded.iter().skip(1).map(|poll| &poll.body).collect();
    let poll_body = |attempt: u8| {
        json(&format!(
            r#"{{"jsonrpc": "2.0", "id": "req-1-poll-{attempt}", "method": "tasks/get", "params": {{"id": "task-1"}}}}"#
        ))
    };
    assert_eq!(polls, [&poll_body(0), &poll_body(1), &poll_body(2)]);
    assert_eq!(
        *transport.pauses.lock().unwrap(),
        [Duration::from_millis(500); 2]
    );
}

#[tokio::test]
async fn pydantic_ai_submit_forwards_the_headers_on_every_poll() {
    let transport = ScriptedTransport::answering(&[SUBMITTED, WORKING, COMPLETED]);
    let headers = BTreeMap::from([
        ("X-Trace".to_owned(), "abc".to_owned()),
        ("Content-Type".to_owned(), "text/plain".to_owned()),
    ]);
    send(&transport, "http://agent.test", &headers)
        .await
        .unwrap();
    let expected = BTreeMap::from([
        ("X-Trace".to_owned(), "abc".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ]);
    let recorded = transport.recorded();
    assert_eq!(recorded.len(), 3);
    let sent: Vec<_> = recorded
        .into_iter()
        .map(|request| request.headers)
        .collect();
    assert_eq!(sent, [expected.clone(), expected.clone(), expected]);
}

#[tokio::test]
async fn pydantic_ai_submit_gives_up_after_thirty_polls() {
    let mut bodies = vec![SUBMITTED];
    bodies.extend([WORKING; 30]);
    let transport = ScriptedTransport::answering(&bodies);
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Task task-1 did not complete within 15.0 seconds"
    );
    let recorded = transport.recorded();
    assert_eq!(recorded.len(), 31);
    assert_eq!(recorded[30].body["id"], json(r#""req-1-poll-29""#));
    assert_eq!(transport.pause_count(), 30);
}

#[rstest]
#[case::failed("failed")]
#[case::canceled("canceled")]
#[tokio::test]
async fn pydantic_ai_submit_fails_on_a_task_that_ended(#[case] state: &str) {
    let ended = format!(r#"{{"result": {{"id": "task-1", "status": {{"state": "{state}"}}}}}}"#);
    let transport = ScriptedTransport::answering(&[SUBMITTED, WORKING, &ended]);
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Task task-1 ended with state: {state}")
    );
    assert_eq!(transport.pause_count(), 1);
}

#[rstest]
#[case::failed("failed")]
#[case::canceled("canceled")]
#[tokio::test]
async fn pydantic_ai_submit_polls_a_first_answer_that_already_ended(#[case] state: &str) {
    let ended = format!(r#"{{"result": {{"id": "task-1", "status": {{"state": "{state}"}}}}}}"#);
    let transport = ScriptedTransport::answering(&[&ended, COMPLETED]);
    let raw = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(raw, json(COMPLETED));
    let recorded = transport.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[1].body["method"], json(r#""tasks/get""#));
    assert_eq!(recorded[1].body["id"], json(r#""req-1-poll-0""#));
    assert_eq!(transport.pause_count(), 0);
}

#[rstest]
#[case::no_result("{}")]
#[case::null_result(r#"{"result": null}"#)]
#[case::empty_task_id(r#"{"result": {"id": "", "status": {"state": "working"}}}"#)]
#[case::null_task_id(r#"{"result": {"id": null, "status": {"state": "working"}}}"#)]
#[case::no_status(r#"{"result": {"kind": "message", "parts": []}}"#)]
#[tokio::test]
async fn pydantic_ai_submit_returns_a_response_without_a_task_id_unpolled(#[case] body: &str) {
    let transport = ScriptedTransport::answering(&[body]);
    let raw = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(raw, json(body));
    assert_eq!(transport.recorded().len(), 1);
    assert_eq!(transport.pause_count(), 0);
}

#[tokio::test]
async fn pydantic_ai_submit_returns_a_completed_response_even_when_it_has_no_result_body() {
    let completed = r#"{"result": {"id": "task-1", "status": {"state": "completed"}}}"#;
    let transport = ScriptedTransport::answering(&[completed]);
    let raw = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(raw, json(completed));
    assert_eq!(transport.recorded().len(), 1);
}

#[tokio::test]
async fn pydantic_ai_submit_accepts_a_completed_poll_without_a_result() {
    let transport = ScriptedTransport::answering(&[
        SUBMITTED,
        r#"{"result": {"status": {"state": "completed"}}}"#,
    ]);
    let raw = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(
        raw,
        json(r#"{"result": {"status": {"state": "completed"}}}"#)
    );
}

#[rstest]
#[case::not_found(404)]
#[case::server_error(500)]
#[case::redirect(302)]
#[tokio::test]
async fn pydantic_ai_submit_fails_on_a_status_that_is_not_a_success(#[case] status: u16) {
    let transport = ScriptedTransport::default();
    transport
        .responses
        .lock()
        .unwrap()
        .push_back(AgentResponse {
            status,
            body: json("{}"),
        });
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::HttpStatus { status: got } if got == status));
}

#[tokio::test]
async fn pydantic_ai_submit_fails_on_a_poll_status_that_is_not_a_success() {
    let transport = ScriptedTransport::answering(&[SUBMITTED]);
    transport
        .responses
        .lock()
        .unwrap()
        .push_back(AgentResponse {
            status: 503,
            body: json("{}"),
        });
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::HttpStatus { status: 503 }));
}

#[rstest]
#[case::list("[1, 2]")]
#[case::text(r#""text""#)]
#[case::null("null")]
#[tokio::test]
async fn pydantic_ai_submit_rejects_a_response_that_is_not_an_object(#[case] body: &str) {
    let transport = ScriptedTransport::answering(&[body]);
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::ResponseNotObject));
}

#[rstest]
#[case::result_is_text(r#"{"result": "done"}"#)]
#[case::status_is_text(r#"{"result": {"status": "done"}}"#)]
#[case::state_is_a_number(r#"{"result": {"status": {"state": 3}}}"#)]
#[tokio::test]
async fn pydantic_ai_submit_names_a_response_field_of_the_wrong_type(#[case] body: &str) {
    let transport = ScriptedTransport::answering(&[body]);
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::Response(_)));
}

#[tokio::test]
async fn pydantic_ai_submit_surfaces_a_transport_failure() {
    let transport = ScriptedTransport::default();
    let error = send(&transport, "http://agent.test", &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::Transport(ScriptExhausted)));
}

#[tokio::test]
async fn pydantic_ai_submit_passes_the_timeout_to_every_request() {
    let transport = ScriptedTransport::answering(&[SUBMITTED, COMPLETED]);
    let params = json(r#"{"message": {"parts": []}}"#);
    let headers = BTreeMap::new();
    send_and_poll_raw(
        &transport,
        &AgentCall {
            api_base: "http://agent.test",
            request_id: "req-1",
            params: &params,
            timeout: Duration::from_millis(2500),
            headers: &headers,
        },
    )
    .await
    .unwrap();
    let timeouts: Vec<_> = transport
        .recorded()
        .iter()
        .map(|request| request.timeout)
        .collect();
    assert_eq!(timeouts, [Duration::from_millis(2500); 2]);
}

pub(crate) struct Counter(pub(crate) u128);

impl EventSource for Counter {
    fn next_uuid(&mut self) -> Uuid {
        self.0 = self.0.saturating_add(1);
        Uuid::from_u128(self.0)
    }

    fn now(&mut self) -> DateTime<Utc> {
        DateTime::UNIX_EPOCH
    }
}
