#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::{ApiError, Method, RunBody, RunContent, RunMessage, call_run};
use crate::watsonx_run::tests::{Fail, Reply, Script, limits};
use crate::watsonx_run::transport::{RunLimits, RunRequest};

const URL: &str = "https://cp4d.example.com/orchestrate/cpd/instances/inst-1/v1/orchestrate/runs";

fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Authorization".to_owned(), "Bearer tok-1".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ])
}

fn body(thread_id: Option<&str>) -> RunBody {
    RunBody {
        agent_id: "agent-1".to_owned(),
        message: RunMessage {
            role: "user",
            content: vec![RunContent {
                response_type: "text",
                text: "hello".to_owned(),
            }],
        },
        thread_id: thread_id.map(str::to_owned),
    }
}

#[tokio::test]
async fn submission_posts_the_run_body_to_the_runs_endpoint() {
    let script = Script::new(vec![Reply::json(
        r#"{"run_id": "run-1", "status": "running"}"#,
    )]);
    let headers = headers();
    let body = body(Some("thread-1"));
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: Some(&body),
        limits: limits(Duration::from_secs(90)),
    };
    let event = call_run(&&script, &request, Method::Post).await.unwrap();
    assert_eq!(event.run_id.as_deref(), Some("run-1"));
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].method, "POST");
    assert_eq!(recorded[0].url, URL);
    assert_eq!(recorded[0].headers, headers);
    assert_eq!(recorded[0].timeout, Duration::from_secs(90));
    assert_eq!(
        recorded[0].body,
        serde_json::from_str::<AgentJson>(
            r#"{"agent_id":"agent-1","message":{"role":"user","content":[{"response_type":"text","text":"hello"}]},"thread_id":"thread-1"}"#
        )
        .unwrap()
    );
}

#[test]
fn submission_leaves_the_thread_id_out_when_it_is_unset() {
    assert_eq!(
        serde_json::to_string(&body(None)).unwrap(),
        r#"{"agent_id":"agent-1","message":{"role":"user","content":[{"response_type":"text","text":"hello"}]}}"#
    );
}

#[rstest]
#[case::ok(200)]
#[case::last_success(299)]
#[tokio::test]
async fn submission_accepts_a_success_status(#[case] status: u16) {
    let script = Script::new(vec![Reply::json("{}").with_status(status)]);
    let headers = headers();
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: None,
        limits: limits(Duration::from_secs(90)),
    };
    call_run(&&script, &request, Method::Post).await.unwrap();
}

#[rstest]
#[case::redirect(301)]
#[case::not_found(404)]
#[case::server_error(500)]
#[tokio::test]
async fn submission_fails_on_a_status_that_is_not_a_success(#[case] status: u16) {
    let script = Script::new(vec![Reply::json("{}").with_status(status)]);
    let headers = headers();
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: None,
        limits: limits(Duration::from_secs(90)),
    };
    let error = call_run(&&script, &request, Method::Post)
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::Status { status: got } if got == status));
}

#[rstest]
#[case::truncated(r#"{"run_id": "#)]
#[case::array("[1, 2]")]
#[tokio::test]
async fn submission_fails_on_a_malformed_answer(#[case] answer: &str) {
    let script = Script::new(vec![Reply::json(answer)]);
    let headers = headers();
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: None,
        limits: limits(Duration::from_secs(90)),
    };
    let error = call_run(&&script, &request, Method::Post)
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::Decode(_)));
}

#[tokio::test]
async fn submission_surfaces_a_transport_failure() {
    let script = Script::new(vec![Reply::failing(Fail::Transport)]);
    let headers = headers();
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: None,
        limits: limits(Duration::from_secs(90)),
    };
    let error = call_run(&&script, &request, Method::Post)
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::Transport(_)));
}

#[test]
fn submission_serializes_the_content_as_a_response_type_and_a_text() {
    assert_eq!(
        serde_json::to_string(&body(None).message.content).unwrap(),
        r#"[{"response_type":"text","text":"hello"}]"#
    );
}

#[tokio::test]
async fn submission_accepts_a_body_at_the_response_limit() {
    let script = Script::new(vec![Reply::json("{}")]);
    let headers = headers();
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: None,
        limits: RunLimits {
            timeout: Duration::from_secs(90),
            max_response_bytes: 2,
        },
    };
    call_run(&&script, &request, Method::Post).await.unwrap();
}

#[tokio::test]
async fn submission_rejects_a_body_past_the_response_limit() {
    let script = Script::new(vec![Reply::json("{}")]);
    let headers = headers();
    let request = RunRequest {
        url: URL,
        headers: &headers,
        body: None,
        limits: RunLimits {
            timeout: Duration::from_secs(90),
            max_response_bytes: 1,
        },
    };
    let error = call_run(&&script, &request, Method::Post)
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::TooLarge { max: 1 }));
}
