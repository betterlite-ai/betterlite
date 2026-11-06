#![cfg(test)]

use axum::http::StatusCode;

use super::async_post_call_success_hook;
use crate::ibm_hooks::config::EventHook;
use crate::ibm_hooks::error::IbmHookError;
use crate::ibm_hooks::response::ModelResponse;
use crate::ibm_hooks::test_support::{
    DetectorServerBody, Mode, capturing_server, config, data, guardrail,
};

const DETECTOR_SERVER_PATH: &str = "/api/v1/text/contents";

const TWO_TEXTS: &str = r#"{"choices":[{"message":{"role":"assistant","content":"hello"}},{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}},{"message":{"role":"assistant","content":"world"}}]}"#;

#[tokio::test]
async fn post_call_checks_every_choice_text() {
    let (base, mut captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, "[[],[]]", 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let response: ModelResponse = serde_json::from_str(TWO_TEXTS).expect("response");

    async_post_call_success_hook(&guardrail, &mut prompt, &response)
        .await
        .expect("hook");

    let body: DetectorServerBody =
        serde_json::from_str(&captured.recv().await.expect("request")).expect("body");
    assert_eq!(body.contents, ["hello".to_owned(), "world".to_owned()]);
    captured.try_recv().unwrap_err();
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}

#[tokio::test]
async fn post_call_skips_a_response_without_text() {
    let (base, mut captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, "[]", 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let response: ModelResponse = serde_json::from_str(
        r#"{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}"#,
    )
    .expect("response");

    async_post_call_success_hook(&guardrail, &mut prompt, &response)
        .await
        .expect("hook");

    captured.try_recv().unwrap_err();
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn post_call_raises_the_violation_message_when_blocking() {
    let response_body = r#"[[{"detection_type":"toxicity","score":0.95,"text":"bad answer"}]]"#;
    let (base, _captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, response_body, 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let response: ModelResponse = serde_json::from_str(
        r#"{"choices":[{"message":{"role":"assistant","content":"bad answer"}}]}"#,
    )
    .expect("response");

    let error = async_post_call_success_hook(&guardrail, &mut prompt, &response)
        .await
        .expect_err("violation");
    assert!(matches!(error, IbmHookError::Violation { .. }));
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn post_call_skips_when_the_event_hook_excludes_it() {
    let (base, mut captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, "[]", 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        Some(vec![EventHook::PreCall]),
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hi"}]}"#);
    let response: ModelResponse =
        serde_json::from_str(r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#)
            .expect("response");

    async_post_call_success_hook(&guardrail, &mut prompt, &response)
        .await
        .expect("hook");

    captured.try_recv().unwrap_err();
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}
