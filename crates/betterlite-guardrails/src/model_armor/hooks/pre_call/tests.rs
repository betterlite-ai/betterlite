#![cfg(test)]

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::model_armor::hooks::error_handling::HookError;
use crate::model_armor::hooks::tests::support::{
    API_ERROR, BLOCKED, CLEAN, NAME, PROMPT, Recorder, SANITIZED, SANITIZED_SAME, SANITIZED_TEXT,
    armor, armor_with, blocked, bucket, data, field, logged, prompt_data, prompt_with_file,
    records, sent_file, settings, text, value,
};
use crate::model_armor::transport::error::TransportError;

#[tokio::test]
async fn pre_call_scans_the_last_user_message_and_records_a_clean_verdict() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
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
        bucket(&request, "metadata").get("_model_armor_response"),
        Some(&value(CLEAN))
    );
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
}

#[tokio::test]
async fn pre_call_blocks_a_flagged_prompt_with_the_400_detail() {
    let fixture = armor_with(vec![(StatusCode::OK, BLOCKED.to_owned())], settings(), true).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (status, detail) = blocked(error);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked by Model Armor"))
    );
    assert!(!detail.contains_key("model_armor_response"));
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("blocked"))
    );
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
}

#[tokio::test]
async fn pre_call_block_detail_carries_the_response_when_not_sanitized() {
    let fixture = armor_with(
        vec![(StatusCode::OK, BLOCKED.to_owned())],
        settings(),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (_, detail) = blocked(error);
    assert_eq!(detail.get("model_armor_response"), Some(&value(BLOCKED)));
}

#[tokio::test]
async fn pre_call_redacts_the_stored_response_when_sanitizing() {
    let fixture = armor_with(
        vec![(StatusCode::OK, SANITIZED_TEXT.to_owned())],
        settings(),
        true,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_response"),
        Some(&value(r#"{"sanitizedText":"[REDACTED]"}"#))
    );
}

#[tokio::test]
async fn pre_call_keeps_the_stored_response_whole_when_not_sanitizing() {
    let fixture = armor_with(
        vec![(StatusCode::OK, SANITIZED_TEXT.to_owned())],
        settings(),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_response"),
        Some(&value(SANITIZED_TEXT))
    );
}

#[tokio::test]
async fn pre_call_masks_the_last_user_message_when_asked() {
    let mut base = settings();
    base.mask_request_content = true;
    let fixture = armor_with(vec![(StatusCode::OK, SANITIZED.to_owned())], base, false).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        request.get("messages"),
        Some(&value(r#"[{"role":"user","content":"masked text"}]"#))
    );
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
}

#[tokio::test]
async fn pre_call_does_not_mask_without_the_setting() {
    let fixture = armor_with(
        vec![(StatusCode::OK, SANITIZED_TEXT.to_owned())],
        settings(),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        request.get("messages"),
        Some(&value(r#"[{"role":"user","content":"hello"}]"#))
    );
}

#[tokio::test]
async fn pre_call_blocks_a_deidentify_match_when_masking_is_off() {
    let fixture = armor_with(
        vec![(StatusCode::OK, SANITIZED.to_owned())],
        settings(),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (status, _detail) = blocked(error);
    assert_eq!(status, 400);
}

#[tokio::test]
async fn pre_call_skips_a_sanitized_text_that_matches_what_it_scanned() {
    let mut base = settings();
    base.mask_request_content = true;
    let fixture = armor_with(
        vec![(StatusCode::OK, SANITIZED_SAME.to_owned())],
        base,
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        request.get("messages"),
        Some(&value(r#"[{"role":"user","content":"hello"}]"#))
    );
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("success"))
    );
}

#[tokio::test]
async fn pre_call_warns_and_skips_without_messages() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = data("{}");
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let log = logged(async {
        hooks
            .pre_call(&mut request, None, &mut recorder.context())
            .await
            .expect("skipped");
    })
    .await;
    assert!(log.contains("No messages in data"));
    assert_eq!(fixture.calls(), 0);
}

#[tokio::test]
async fn pre_call_skips_a_last_user_message_without_text() {
    let fixture = armor(Vec::new()).await;
    let hooks = fixture.hooks();
    let mut request = data(r#"{"messages":[{"role":"user","content":""}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    assert!(
        bucket(&request, "metadata")
            .get("_model_armor_response")
            .is_none()
    );
}

#[tokio::test]
async fn pre_call_scans_files_before_the_text() {
    let fixture = armor(vec![
        (StatusCode::OK, CLEAN.to_owned()),
        (StatusCode::OK, CLEAN.to_owned()),
    ])
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_with_file();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(fixture.calls(), 2);
    assert_eq!(sent_file(&fixture.request(0)), "aGVsbG8=");
    assert_eq!(fixture.request_text(1), PROMPT);
    match bucket(&request, "metadata").get("_model_armor_response") {
        Some(GuardrailValue::Array(items)) => assert_eq!(items.len(), 2),
        other => panic!("expected two scan responses, got {other:?}"),
    }
}

#[tokio::test]
async fn pre_call_a_file_block_stops_before_the_text_scan() {
    let fixture = armor(vec![(StatusCode::OK, BLOCKED.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_with_file();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("blocked");
    blocked(error);
    assert_eq!(fixture.calls(), 1);
    assert_eq!(
        bucket(&request, "metadata").get("_model_armor_status"),
        Some(&text("blocked"))
    );
}

#[tokio::test]
async fn pre_call_a_second_hook_appends_to_the_stored_response() {
    let fixture = armor(vec![
        (StatusCode::OK, CLEAN.to_owned()),
        (StatusCode::OK, CLEAN.to_owned()),
    ])
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("clean");
    let mut again = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut again.context())
        .await
        .expect("clean");
    assert_eq!(fixture.calls(), 2);
    match bucket(&request, "metadata").get("_model_armor_response") {
        Some(GuardrailValue::Array(items)) => assert_eq!(items.len(), 2),
        other => panic!("expected two scan responses, got {other:?}"),
    }
}

#[tokio::test]
async fn pre_call_api_errors_stop_the_hook_when_fail_closed() {
    let fixture = armor(vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("fail closed");
    assert!(matches!(error, HookError::Api(_)));
    assert!(!request.contains_key("metadata"));
}

#[tokio::test]
async fn pre_call_api_errors_continue_when_fail_open() {
    let fixture = armor(vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())]).await;
    let hooks = fixture.hooks_fail_open();
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    hooks
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect("fail open");
    let entries = records(&request, "metadata");
    assert_eq!(entries.len(), 1);
    assert_eq!(field(&entries[0], "guardrail_status"), text("success"));
}

#[tokio::test]
async fn pre_call_a_response_that_is_not_json_follows_fail_on_error() {
    let closed = armor(vec![(StatusCode::OK, "not json".to_owned())]).await;
    let mut request = prompt_data();
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    let error = closed
        .hooks()
        .pre_call(&mut request, None, &mut recorder.context())
        .await
        .expect_err("fail closed");
    assert!(matches!(
        error,
        HookError::Transport(TransportError::ResponseJson(_))
    ));
    let open = armor(vec![(StatusCode::OK, "not json".to_owned())]).await;
    let mut open_request = prompt_data();
    let mut open_recorder = Recorder::new(Some(GuardrailEventHooks::PreCall));
    open.hooks_fail_open()
        .pre_call(&mut open_request, None, &mut open_recorder.context())
        .await
        .expect("fail open");
    assert_eq!(records(&open_request, "metadata").len(), 1);
}
