#![cfg(test)]

use axum::http::StatusCode;

use super::async_moderation_hook;
use crate::ibm_hooks::config::EventHook;
use crate::ibm_hooks::error::IbmHookError;
use crate::ibm_hooks::test_support::{Mode, capturing_server, config, data, guardrail};

const DETECTOR_SERVER_PATH: &str = "/api/v1/text/contents";

#[tokio::test]
async fn moderation_hook_raises_when_blocking_and_violations_found() {
    let response = r#"[[{"detection_type":"jailbreak","score":0.9,"text":"bad"}]]"#;
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

    let error = async_moderation_hook(&guardrail, &mut prompt)
        .await
        .expect_err("violation");
    assert!(matches!(error, IbmHookError::Violation { .. }));
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn moderation_hook_records_the_guardrail_when_blocking_is_disabled() {
    let response = r#"[[{"detection_type":"jailbreak","score":0.9,"text":"bad"}]]"#;
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

    async_moderation_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}

#[tokio::test]
async fn moderation_hook_skips_when_the_event_hook_excludes_it() {
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
    let mut prompt = data(r#"{"messages":[{"role":"user","content":"hello"}]}"#);

    async_moderation_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    captured.try_recv().unwrap_err();
    assert!(prompt.metadata.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn moderation_hook_leaves_the_request_messages_untouched() {
    let (base, _captured, mut server) =
        capturing_server(DETECTOR_SERVER_PATH, StatusCode::OK, "[]", 4).await;
    let guardrail = guardrail(
        config(
            Mode::DetectorServer,
            &format!("{base}{DETECTOR_SERVER_PATH}"),
            true,
        ),
        Some(vec![EventHook::DuringCall]),
    );
    let mut prompt = data(
        r#"{"messages":[{"role":"user","content":"first"},{"role":"user","content":[{"type":"text","text":"second"}]}]}"#,
    );
    let original = prompt.messages.clone();

    async_moderation_hook(&guardrail, &mut prompt)
        .await
        .expect("hook");

    assert_eq!(prompt.messages, original);
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
    server.shutdown().await;
}
