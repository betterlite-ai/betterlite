use axum::http::StatusCode;

use super::support::{
    AimMode, aim_guardrail, chat_data, chat_response, content_of, message, serve_aim,
};
use crate::aim::error::{AimError, CONTENT_POLICY_VIOLATION};
use crate::lasso_masking::wire::ModelResponseShape;
use crate::third_party_hooks_a::aim;
use crate::third_party_hooks_a::error::ThirdPartyHookError;

const CONTENT_REDACTED: &str = "redacted:first";
const CONTENT_KEPT: &str = "redacted:second";

#[tokio::test]
async fn aim_skips_embeddings_unless_inspect_embeddings_is_set() {
    let (base, _server, captured) = serve_aim(AimMode::Monitor).await;
    let guardrail = aim_guardrail(&base, false);
    let mut data = chat_data(&[message("user", "hello")]);
    let before = data.clone();
    aim::hook(
        &guardrail,
        &mut data,
        aim::AimRequestHook::PreCall,
        "embedding",
        None,
    )
    .await
    .expect("pre_call");
    aim::hook(
        &guardrail,
        &mut data,
        aim::AimRequestHook::Moderation,
        "embedding",
        None,
    )
    .await
    .expect("moderation");
    assert_eq!(data, before);
    assert!(captured.lock().expect("captured").is_empty());
}

#[tokio::test]
async fn aim_inspects_embeddings_when_inspect_embeddings_is_set() {
    let (base, _server, captured) = serve_aim(AimMode::Monitor).await;
    let guardrail = aim_guardrail(&base, true);
    let mut data = chat_data(&[message("user", "hello")]);
    aim::hook(
        &guardrail,
        &mut data,
        aim::AimRequestHook::PreCall,
        "aembedding",
        None,
    )
    .await
    .expect("pre_call");
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "/fw/v1/analyze");
    assert_eq!(
        calls[0]
            .headers
            .get("x-aim-litellm-hook")
            .map(String::as_str),
        Some("pre_call")
    );
    assert_eq!(
        calls[0].headers.get("authorization").map(String::as_str),
        Some("Bearer aim-key")
    );
    assert!(!calls[0].headers.contains_key("x-aim-gateway-key-alias"));
}

#[tokio::test]
async fn aim_pre_call_writes_the_anonymized_data_back() {
    let (base, _server, _captured) = serve_aim(AimMode::Redact).await;
    let guardrail = aim_guardrail(&base, false);
    let mut data = chat_data(&[message("user", "hello")]);
    aim::hook(
        &guardrail,
        &mut data,
        aim::AimRequestHook::PreCall,
        "completion",
        None,
    )
    .await
    .expect("pre_call");
    assert_eq!(data, chat_data(&[message("user", "redacted:hello")]));
}

#[tokio::test]
async fn aim_moderation_keeps_the_request_data_read_only() {
    let (base, _server, captured) = serve_aim(AimMode::Redact).await;
    let guardrail = aim_guardrail(&base, false);
    let mut data = chat_data(&[message("system", "be precise"), message("user", "hello")]);
    let before = data.clone();
    aim::hook(
        &guardrail,
        &mut data,
        aim::AimRequestHook::Moderation,
        "completion",
        Some("alias-1"),
    )
    .await
    .expect("moderation");
    assert_eq!(data, before);
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]
            .headers
            .get("x-aim-litellm-hook")
            .map(String::as_str),
        Some("moderation")
    );
    assert_eq!(
        calls[0]
            .headers
            .get("x-aim-gateway-key-alias")
            .map(String::as_str),
        Some("alias-1")
    );
}

#[tokio::test]
async fn aim_post_call_blocks_with_the_detection_message() {
    let (base, _server, _captured) = serve_aim(AimMode::Block).await;
    let guardrail = aim_guardrail(&base, false);
    let data = chat_data(&[message("user", "hello")]);
    let mut response = chat_response(&["first", "second"]);
    let error = aim::post_call_success(&guardrail, &data, &mut response, None)
        .await
        .expect_err("blocked");
    assert!(matches!(
        &error,
        ThirdPartyHookError::Aim(AimError::Rejected { message, code })
            if message == "AIM says no" && *code == CONTENT_POLICY_VIOLATION
    ));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn aim_post_call_continues_when_a_block_carries_no_message() {
    let (base, _server, captured) = serve_aim(AimMode::BlockSilently).await;
    let guardrail = aim_guardrail(&base, false);
    let data = chat_data(&[message("user", "hello")]);
    let mut response = chat_response(&["first"]);
    aim::post_call_success(&guardrail, &data, &mut response, None)
        .await
        .expect("post_call");
    assert_eq!(content_of(&response, 0).as_deref(), Some("first"));
    assert_eq!(captured.lock().expect("captured").len(), 1);
}

#[tokio::test]
async fn aim_post_call_writes_each_redacted_output_back() {
    let (base, _server, captured) = serve_aim(AimMode::Redact).await;
    let guardrail = aim_guardrail(&base, false);
    let data = chat_data(&[message("user", "hello")]);
    let mut response = chat_response(&["first", "second"]);
    aim::post_call_success(&guardrail, &data, &mut response, None)
        .await
        .expect("post_call");
    assert_eq!(content_of(&response, 0), Some(CONTENT_REDACTED.to_owned()));
    assert_eq!(content_of(&response, 1), Some(CONTENT_KEPT.to_owned()));
    let calls = captured.lock().expect("captured");
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| {
        call.headers.get("x-aim-litellm-hook").map(String::as_str) == Some("output")
    }));
}

#[tokio::test]
async fn aim_post_call_ignores_choices_without_a_message() {
    let (base, _server, captured) = serve_aim(AimMode::Monitor).await;
    let guardrail = aim_guardrail(&base, false);
    let data = chat_data(&[message("user", "hello")]);
    let mut response: ModelResponseShape =
        serde_json::from_str(r#"{"choices":[{}]}"#).expect("response");
    aim::post_call_success(&guardrail, &data, &mut response, None)
        .await
        .expect("post_call");
    assert!(captured.lock().expect("captured").is_empty());
}
