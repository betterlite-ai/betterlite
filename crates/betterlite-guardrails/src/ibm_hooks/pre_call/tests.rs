#![cfg(test)]

use axum::http::StatusCode;

use super::async_pre_call_hook;
use crate::ibm_detector_client::message::{detector_server_message, orchestrator_message};
use crate::ibm_detector_client::wire::{Detection, OrchestratorResponse};
use crate::ibm_hooks::config::EventHook;
use crate::ibm_hooks::error::IbmHookError;
use crate::ibm_hooks::test_support::{
    DetectorServerBody, Mode, OrchestratorBody, capturing_server, config, data, guardrail,
};

const DETECTOR_SERVER_PATH: &str = "/api/v1/text/contents";
const ORCHESTRATOR_PATH: &str = "/api/v2/text/detection/content";

#[tokio::test]
async fn pre_call_batches_every_message_text_in_one_detector_server_request() {
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
    let mut prompt = data(
        r#"{"messages":[{"role":"user","content":"first message"},{"role":"assistant","content":[{"type":"text","text":"second message"},{"type":"image_url","image_url":{"url":"https://example.test/pixel.png"}}]}]}"#,
    );

    async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    let body: DetectorServerBody =
        serde_json::from_str(&captured.recv().await.expect("request")).expect("body");
    assert_eq!(
        body.contents,
        ["first message".to_owned(), "second message".to_owned()]
    );
    captured.try_recv().unwrap_err();
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_calls_the_orchestrator_once_per_text() {
    let (base, mut captured, mut server) =
        capturing_server(ORCHESTRATOR_PATH, StatusCode::OK, r#"{"detections":[]}"#, 4).await;
    let guardrail = guardrail(
        config(
            Mode::Orchestrator,
            &format!("{base}{ORCHESTRATOR_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(
        r#"{"messages":[{"role":"user","content":"first"},{"role":"user","content":"second"}]}"#,
    );

    async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    let first: OrchestratorBody =
        serde_json::from_str(&captured.recv().await.expect("first")).expect("body");
    let second: OrchestratorBody =
        serde_json::from_str(&captured.recv().await.expect("second")).expect("body");
    assert_eq!(first.content, "first");
    assert_eq!(second.content, "second");
    captured.try_recv().unwrap_err();
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_orchestrator_raises_the_violation_message_when_blocking() {
    let response = r#"{"detections":[{"detection_type":"jailbreak","score":0.9,"text":"bad"}]}"#;
    let (base, mut captured, mut server) =
        capturing_server(ORCHESTRATOR_PATH, StatusCode::OK, response, 4).await;
    let guardrail = guardrail(
        config(
            Mode::Orchestrator,
            &format!("{base}{ORCHESTRATOR_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(
        r#"{"messages":[{"role":"user","content":"first"},{"role":"user","content":"second"}]}"#,
    );

    let error = async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect_err("violation");
    let IbmHookError::Violation { message } = error else {
        panic!("expected a violation");
    };
    let parsed: OrchestratorResponse = serde_json::from_str(response).expect("detections");
    assert_eq!(
        message,
        orchestrator_message(&guardrail.client.config, &parsed.detections)
    );
    let first: OrchestratorBody =
        serde_json::from_str(&captured.try_recv().expect("first request")).expect("body");
    assert_eq!(first.content, "first");
    captured.try_recv().unwrap_err();
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_orchestrator_records_the_guardrail_when_blocking_is_disabled() {
    let response = r#"{"detections":[{"detection_type":"jailbreak","score":0.9,"text":"bad"}]}"#;
    let (base, mut captured, mut server) =
        capturing_server(ORCHESTRATOR_PATH, StatusCode::OK, response, 4).await;
    let guardrail = guardrail(
        config(
            Mode::Orchestrator,
            &format!("{base}{ORCHESTRATOR_PATH}"),
            false,
        ),
        None,
    );
    let mut prompt = data(
        r#"{"messages":[{"role":"user","content":"first"},{"role":"user","content":"second"}]}"#,
    );

    async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    let first: OrchestratorBody =
        serde_json::from_str(&captured.try_recv().expect("first request")).expect("body");
    let second: OrchestratorBody =
        serde_json::from_str(&captured.try_recv().expect("second request")).expect("body");
    assert_eq!(first.content, "first");
    assert_eq!(second.content, "second");
    captured.try_recv().unwrap_err();
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_raises_the_violation_message_when_blocking() {
    let response = r#"[[{"detection_type":"toxic","score":0.9,"text":"bad"}],[]]"#;
    let (base, _captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, response, 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);

    let error = async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect_err("violation");
    let IbmHookError::Violation { message } = error else {
        panic!("expected a violation");
    };
    let detections: Vec<Vec<Detection>> = serde_json::from_str(response).expect("detections");
    assert_eq!(
        message,
        detector_server_message(&guardrail.client.config, &detections)
    );
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_records_the_guardrail_when_blocking_is_disabled() {
    let response = r#"[[{"detection_type":"toxic","score":0.9,"text":"bad"}]]"#;
    let (base, _captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, response, 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            false,
        ),
        None,
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);

    async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_skips_when_the_event_hook_excludes_it() {
    let (base, mut captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, "[]", 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        Some(vec![EventHook::PostCall]),
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hello"}]}"#);

    async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    captured.try_recv().unwrap_err();
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_propagates_a_detector_error() {
    let (base, _captured, mut server) = capturing_server(
        DETECTOR_SERVER_PATH,
        StatusCode::INTERNAL_SERVER_ERROR,
        "boom",
        4,
    )
    .await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        None,
    );
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hello"}]}"#);

    let error = async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect_err("detector error");
    assert!(matches!(error, IbmHookError::Detector(_)));
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_records_the_guardrail_without_text_and_skips_the_detector() {
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
    let mut prompt = data(r#"{"messages":[{"role":"assistant","content":""}]}"#);

    async_pre_call_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    captured.try_recv().unwrap_err();
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}
