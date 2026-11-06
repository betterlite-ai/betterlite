#![cfg(test)]

use std::num::NonZeroUsize;
use std::time::Duration;

use rstest::rstest;

use crate::pydantic_ai_transformation::fake_streaming::StreamPacing;
use crate::watsonx_credentials::params::WxoLitellmParams;
use crate::watsonx_run::handler::{RunError, STREAMING_TIMEOUT};
use crate::watsonx_run::run_api::ApiError;
use crate::watsonx_run::tests::{
    Fail, Reply, Script, ScriptError, litellm_params, run_handler, send_params, token_reply,
};
use crate::watsonx_transform::extract::WxoSendParams;
use crate::watsonx_transform::fake_streaming::{WxoStreamResult, WxoStreamStep};

const RUN: &str = "https://cp4d.example.com/orchestrate/cpd/instances/inst-1/v1/orchestrate/runs";
const STREAM: &str =
    "https://cp4d.example.com/orchestrate/cpd/instances/inst-1/v1/orchestrate/runs/stream";
const POLLED_RUN: &str =
    "https://cp4d.example.com/orchestrate/cpd/instances/inst-1/v1/orchestrate/runs/run-1";

fn script_text(steps: &[WxoStreamStep]) -> String {
    let mut text = String::new();
    for step in steps {
        if let WxoStreamResult::Artifact(update) = &step.event.result {
            for part in &update.artifact.parts {
                text.push_str(part.text.as_deref().unwrap_or_default());
            }
        }
    }
    text
}

fn sse_text_step(text: &str) -> String {
    format!("data: {{\"last_message\": {{\"content\": [{{\"text\": \"{text}\"}}]}}}}")
}

#[tokio::test]
async fn handler_submits_polls_and_returns_the_a2a_message() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(r#"{"run_id": "run-1", "status": "running"}"#),
        Reply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "hi there"}]}}"#,
        ),
    ]);
    let mut handler = run_handler(&script);
    let response = handler
        .handle_non_streaming("req-1", &send_params(), &litellm_params())
        .await
        .unwrap();
    assert_eq!(response.id, "req-1");
    assert_eq!(response.result.kind, "message");
    assert_eq!(response.result.role, "agent");
    assert_eq!(response.result.parts[0].text.as_deref(), Some("hi there"));
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].method, "POST");
    assert_eq!(recorded[0].url, RUN);
    assert_eq!(recorded[0].timeout, Duration::from_secs(90));
    assert_eq!(recorded[0].headers["Authorization"], "Bearer tok-1");
    assert_eq!(recorded[0].headers["Content-Type"], "application/json");
    assert_eq!(recorded[0].headers["Accept"], "application/json");
    assert_eq!(recorded[0].body["agent_id"], "agent-1");
    assert_eq!(recorded[0].body["message"]["role"], "user");
    assert_eq!(recorded[0].body["message"]["content"][0]["text"], "hello");
    assert_eq!(recorded[0].body["thread_id"], "thread-1");
    assert_eq!(recorded[1].method, "GET");
    assert_eq!(recorded[1].url, POLLED_RUN);
    assert_eq!(recorded[1].timeout, Duration::from_secs(90));
}

#[tokio::test]
async fn handler_joins_the_text_of_every_message_part() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "ok"}]}}"#,
        ),
    ]);
    let params: WxoSendParams = serde_json::from_str(
        r#"{"message": {"parts": [{"kind": "text", "text": "one"}, {"text": "two"}, {"kind": "image", "text": "skip"}]}}"#,
    )
    .unwrap();
    run_handler(&script)
        .handle_non_streaming("req-1", &params, &litellm_params())
        .await
        .unwrap();
    assert_eq!(
        script.recorded()[0].body["message"]["content"][0]["text"],
        "one two"
    );
}

#[tokio::test]
async fn handler_strips_the_trailing_slashes_of_the_cp4d_host() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "ok"}]}}"#,
        ),
    ]);
    run_handler(&script)
        .handle_non_streaming("req-1", &send_params(), &litellm_params())
        .await
        .unwrap();
    assert_eq!(script.recorded()[0].url, RUN);
}

#[tokio::test]
async fn handler_reports_a_missing_required_litellm_param() {
    let script = Script::new(Vec::new());
    let raw = WxoLitellmParams {
        cp4d_host: String::new(),
        ..litellm_params()
    };
    let error: RunError<ScriptError, ScriptError> = run_handler(&script)
        .handle_non_streaming("req-1", &send_params(), &raw)
        .await
        .unwrap_err();
    assert!(matches!(error, RunError::Params(_)));
    assert_eq!(script.recorded().len(), 0);
}

#[tokio::test]
async fn handler_reports_an_authentication_failure() {
    let script = Script::new(vec![Reply::json("{}").with_status(401)]);
    let error: RunError<ScriptError, ScriptError> = run_handler(&script)
        .handle_non_streaming("req-1", &send_params(), &litellm_params())
        .await
        .unwrap_err();
    assert!(matches!(error, RunError::Credentials(_)));
    assert_eq!(script.recorded().len(), 0);
}

#[tokio::test]
async fn handler_uses_a_ninety_second_timeout_for_the_non_streaming_flow() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(r#"{"run_id": "run-1", "status": "running"}"#),
        Reply::json(r#"{"status": "completed"}"#),
    ]);
    run_handler(&script)
        .handle_non_streaming("req-1", &send_params(), &litellm_params())
        .await
        .unwrap();
    let timeouts: Vec<_> = script
        .recorded()
        .iter()
        .map(|request| request.timeout)
        .collect();
    assert_eq!(timeouts, [Duration::from_secs(90); 2]);
}

#[tokio::test]
async fn handler_uses_a_hundred_and_twenty_second_timeout_for_the_streaming_flow() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(r#"{"run_id": "run-1", "status": "running"}"#),
        Reply::json(r#"{"status": "completed"}"#),
    ]);
    run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    let timeouts: Vec<_> = script
        .recorded()
        .iter()
        .map(|request| request.timeout)
        .collect();
    assert_eq!(timeouts, [STREAMING_TIMEOUT; 2]);
}

#[tokio::test]
async fn streaming_accumulates_the_text_of_every_sse_event() {
    let script = Script::new(vec![
        token_reply(),
        Reply::sse(&[
            &sse_text_step("Hi "),
            &sse_text_step("there"),
            "data: [DONE]",
        ]),
    ]);
    let steps = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    assert_eq!(script_text(&steps), "Hi there");
    assert!(matches!(steps[0].event.result, WxoStreamResult::Task(_)));
    assert!(matches!(steps[1].event.result, WxoStreamResult::Status(_)));
    assert!(matches!(
        steps.last().unwrap().event.result,
        WxoStreamResult::Status(_)
    ));
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, STREAM);
    assert_eq!(
        recorded[0].headers["Accept"],
        "text/event-stream, application/json"
    );
    assert_eq!(recorded[0].timeout, STREAMING_TIMEOUT);
}

#[rstest]
#[case::upper("TEXT/EVENT-STREAM")]
#[case::mixed("Text/Event-Stream")]
#[tokio::test]
async fn streaming_detects_the_event_stream_content_type_ignoring_case(#[case] content_type: &str) {
    let script = Script::new(vec![
        token_reply(),
        Reply::sse(&[&sse_text_step("sse")]).with_content_type(content_type),
    ]);
    let steps = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    assert_eq!(script_text(&steps), "sse");
    assert_eq!(script.recorded().len(), 1);
}

#[tokio::test]
async fn streaming_detects_the_event_stream_content_type_with_parameters() {
    let script = Script::new(vec![token_reply(), Reply::sse(&[&sse_text_step("sse")])]);
    let steps = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    assert_eq!(script_text(&steps), "sse");
    assert_eq!(script.recorded().len(), 1);
}

#[rstest]
#[case::json("application/json")]
#[case::missing("")]
#[tokio::test]
async fn streaming_polls_when_the_content_type_is_not_event_stream(#[case] content_type: &str) {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(r#"{"run_id": "run-1", "status": "running"}"#).with_content_type(content_type),
        Reply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "polled"}]}}"#,
        ),
    ]);
    let steps = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    assert_eq!(script_text(&steps), "polled");
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].url, STREAM);
    assert_eq!(recorded[1].method, "GET");
    assert_eq!(recorded[1].url, POLLED_RUN);
    assert_eq!(recorded[1].timeout, STREAMING_TIMEOUT);
}

#[tokio::test]
async fn fallback_submits_a_non_streaming_run_when_the_stream_post_fails() {
    let script = Script::new(vec![
        token_reply(),
        Reply::failing(Fail::Transport),
        Reply::json(
            r#"{"run_id": "run-1", "status": "completed", "last_message": {"content": [{"text": "fallback"}]}}"#,
        ),
    ]);
    let steps = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    assert_eq!(script_text(&steps), "fallback");
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].url, STREAM);
    assert_eq!(recorded[0].timeout, STREAMING_TIMEOUT);
    assert_eq!(recorded[1].url, RUN);
    assert_eq!(recorded[1].method, "POST");
    assert_eq!(recorded[1].timeout, Duration::from_secs(90));
    assert_eq!(recorded[1].headers["Accept"], "application/json");
    assert_eq!(*script.token_calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn fallback_does_not_submit_when_the_stream_status_is_an_error() {
    let script = Script::new(vec![token_reply(), Reply::json("{}").with_status(500)]);
    let error: RunError<ScriptError, ScriptError> = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RunError::Api(ApiError::Status { status: 500 })
    ));
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, STREAM);
}

#[tokio::test]
async fn fallback_does_not_submit_when_the_stream_body_cannot_be_read() {
    let script = Script::new(vec![token_reply(), Reply::failing(Fail::Read)]);
    let error: RunError<ScriptError, ScriptError> = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, RunError::Body(_)));
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, STREAM);
}

#[tokio::test]
async fn fallback_does_not_submit_when_the_stream_answer_is_malformed() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(r#""not json""#).with_content_type("application/json"),
    ]);
    let error: RunError<ScriptError, ScriptError> = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, RunError::Api(ApiError::Decode(_))));
    let recorded = script.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, STREAM);
}

#[tokio::test]
async fn handler_reports_a_stream_success_without_text_as_an_empty_answer() {
    let script = Script::new(vec![
        token_reply(),
        Reply::json(r#"{"run_id": "run-1", "status": "completed"}"#),
    ]);
    let steps = run_handler(&script)
        .handle_streaming(
            "req-1",
            &send_params(),
            &litellm_params(),
            StreamPacing::default(),
        )
        .await
        .unwrap();
    assert_eq!(script_text(&steps), "");
    assert_eq!(steps.len(), 4);
}

#[tokio::test]
async fn handler_keeps_the_pacing_it_was_given() {
    let script = Script::new(vec![token_reply(), Reply::sse(&[&sse_text_step("abcdef")])]);
    let pacing = StreamPacing {
        chunk_size: NonZeroUsize::new(4).unwrap(),
        delay: Duration::from_millis(25),
    };
    let steps = run_handler(&script)
        .handle_streaming("req-1", &send_params(), &litellm_params(), pacing)
        .await
        .unwrap();
    let artifacts = steps
        .iter()
        .filter(|step| matches!(step.event.result, WxoStreamResult::Artifact(_)))
        .count();
    assert_eq!(artifacts, 2);
    assert_eq!(steps[1].delay_after, Duration::from_millis(25));
}
