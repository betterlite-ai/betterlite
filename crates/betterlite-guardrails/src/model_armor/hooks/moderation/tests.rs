#![cfg(test)]

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::model_armor::hooks::error_handling::HookError;
use crate::model_armor::hooks::tests::support::{
    API_ERROR, BLOCKED, CLEAN, NAME, PROMPT, Recorder, SANITIZED, SANITIZED_TEXT, armor_with,
    blocked, bucket, data, field, logged, prompt_data, prompt_with_file, records, sent_file,
    settings_for, text, value,
};

#[tokio::test]
async fn moderation_scans_the_prompt_for_a_during_call() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    hooks
        .moderation(&mut request, Some("completion"), &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(fixture.calls(), 1);
    assert!(fixture.request(0).path.ends_with(":sanitizeUserPrompt"));
    assert_eq!(fixture.request_text(0), PROMPT);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
    let entries = records(&request, "metadata");
    assert_eq!(field(&entries[0], "guardrail_mode"), text("during_call"));
}

#[tokio::test]
async fn moderation_blocks_a_flagged_prompt() {
    let fixture = armor_with(
        vec![(StatusCode::OK, BLOCKED.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    let error = hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (status, detail) = blocked(error);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked by Model Armor"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("blocked"))
    );
}

#[tokio::test]
async fn moderation_masks_the_last_user_message() {
    let mut base = settings_for(&["during_call"]);
    base.mask_request_content = true;
    let fixture = armor_with(vec![(StatusCode::OK, SANITIZED.to_owned())], base, false).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        request.get("messages"),
        Some(&value(r#"[{"role":"user","content":"masked text"}]"#))
    );
}

#[tokio::test]
async fn moderation_runs_only_for_mcp_calls_when_configured() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["during_mcp_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringMcpCall));
    hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    let mut mcp = Recorder::new(Some(GuardrailEventHooks::DuringMcpCall));
    hooks
        .moderation(&mut request, Some("call_mcp_tool"), &mut mcp.context())
        .await
        .expect("runs");
    assert_eq!(fixture.calls(), 1);
}

#[tokio::test]
async fn moderation_records_the_file_scan_first() {
    let fixture = armor_with(
        vec![
            (StatusCode::OK, CLEAN.to_owned()),
            (StatusCode::OK, CLEAN.to_owned()),
        ],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_with_file();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(fixture.calls(), 2);
    assert_eq!(sent_file(&fixture.request(0)), "aGVsbG8=");
    assert_eq!(fixture.request_text(1), PROMPT);
}

#[tokio::test]
async fn moderation_api_errors_stop_the_hook_when_fail_closed() {
    let fixture = armor_with(
        vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    let error = hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect_err("fail closed");
    assert!(matches!(error, HookError::Api(_)));
    assert!(!request.contains_key("metadata"));
}

#[tokio::test]
async fn moderation_api_errors_continue_when_fail_open() {
    let fixture = armor_with(
        vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks_fail_open();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect("fail open");
    assert_eq!(records(&request, "metadata").len(), 1);
}

#[tokio::test]
async fn moderation_warns_without_messages() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = data("{}");
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    let log = logged(async {
        hooks
            .moderation(&mut request, None, &mut recorder.context())
            .await
            .expect("skipped");
    })
    .await;
    assert!(log.contains("No messages in data"));
    assert_eq!(fixture.calls(), 0);
}

#[tokio::test]
async fn moderation_does_not_mask_without_the_setting() {
    let fixture = armor_with(
        vec![(StatusCode::OK, SANITIZED_TEXT.to_owned())],
        settings_for(&["during_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::DuringCall));
    hooks
        .moderation(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        request.get("messages"),
        Some(&value(r#"[{"role":"user","content":"hello"}]"#))
    );
}
