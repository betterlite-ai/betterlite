#![cfg(test)]

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::model_armor::decisions::model_response::CallResponse;
use crate::model_armor::hooks::error_handling::HookError;
use crate::model_armor::hooks::tests::support::{
    API_ERROR, BLOCKED, CLEAN, NAME, Recorder, SANITIZED, SANITIZED_TEXT, armor, armor_with,
    blocked, bucket, contents, field, model_response, prompt_data, records, settings, settings_for,
    standard_records, text, value,
};

#[tokio::test]
async fn post_call_blocks_a_flagged_response_with_the_400_detail() {
    let fixture = armor(vec![(StatusCode::OK, BLOCKED.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    let error = hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (status, detail) = blocked(error);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Response blocked by Model Armor"))
    );
    assert_eq!(fixture.calls(), 1);
    assert!(fixture.request(0).path.ends_with(":sanitizeModelResponse"));
    assert_eq!(fixture.request_text(0), "hello");
    assert_eq!(contents(&response), vec!["hello".to_owned()]);
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
    let entries = standard_records(&recorder);
    assert_eq!(entries.len(), 1);
    assert_eq!(field(&entries[0], "guardrail_status"), text("success"));
    assert_eq!(
        field(
            &field(&entries[0], "guardrail_response"),
            "model_armor_status"
        ),
        text("blocked")
    );
}

#[tokio::test]
async fn post_call_records_the_standard_logging_entry() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    request.insert("start_time".to_owned(), GuardrailValue::Decimal(1.5));
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("clean");
    let entries = standard_records(&recorder);
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(field(entry, "guardrail_name"), text(NAME));
    assert_eq!(field(entry, "guardrail_provider"), text("model_armor"));
    assert_eq!(field(entry, "guardrail_mode"), text("post_call"));
    assert_eq!(field(entry, "guardrail_status"), text("success"));
    assert_eq!(field(entry, "start_time"), GuardrailValue::Decimal(1.5));
    assert_eq!(field(entry, "end_time"), GuardrailValue::Null);
    assert_eq!(field(entry, "duration"), GuardrailValue::Null);
    assert_eq!(field(entry, "masked_entity_count"), GuardrailValue::Null);
    let logged = field(entry, "guardrail_response");
    assert_eq!(field(&logged, "model_armor_response"), value(CLEAN));
    assert_eq!(field(&logged, "model_armor_status"), text("success"));
}

#[tokio::test]
async fn post_call_masks_the_response_choices() {
    let mut base = settings();
    base.mask_response_content = true;
    let fixture = armor_with(vec![(StatusCode::OK, SANITIZED.to_owned())], base, false).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(
        r#"{"choices":[{"message":{"content":"hello"}},{"message":{"content":"second"}},{"message":{"content":""}}]}"#,
    );
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(
        contents(&response),
        vec![
            "masked text".to_owned(),
            "masked text".to_owned(),
            String::new(),
        ]
    );
    assert_eq!(
        bucket(&request, "metadata").get("applied_guardrails"),
        Some(&GuardrailValue::Array(vec![text(NAME)]))
    );
}

#[tokio::test]
async fn post_call_keeps_a_streaming_choice() {
    let mut base = settings();
    base.mask_response_content = true;
    let fixture = armor_with(vec![(StatusCode::OK, SANITIZED.to_owned())], base, false).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"delta":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(contents(&response), vec!["hello".to_owned()]);
    assert_eq!(fixture.calls(), 1);
}

#[tokio::test]
async fn post_call_skips_a_non_chat_response() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = CallResponse::Other;
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    assert!(standard_records(&recorder).is_empty());
    assert_eq!(records(&request, "metadata").len(), 1);
}

#[tokio::test]
async fn post_call_skips_an_empty_response() {
    let fixture = armor(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"message":{"content":""}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    assert!(standard_records(&recorder).is_empty());
    assert_eq!(records(&request, "metadata").len(), 1);
}

#[tokio::test]
async fn post_call_skips_when_not_configured_for_post_call() {
    let fixture = armor_with(
        vec![(StatusCode::OK, CLEAN.to_owned())],
        settings_for(&["pre_call"]),
        false,
    )
    .await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(
        r#"
        {"choices":[{"message":{"content":"hello"}}]}
    "#,
    );
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("skipped");
    assert_eq!(fixture.calls(), 0);
    assert_eq!(contents(&response), vec!["hello".to_owned()]);
    assert_eq!(records(&request, "metadata").len(), 1);
}

#[tokio::test]
async fn post_call_api_errors_stop_the_hook_when_fail_closed() {
    let fixture = armor(vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    let error = hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect_err("fail closed");
    assert!(matches!(error, HookError::Api(_)));
    assert!(request.get("metadata").is_none());
    assert!(standard_records(&recorder).is_empty());
}

#[tokio::test]
async fn post_call_api_errors_continue_when_fail_open() {
    let fixture = armor(vec![(StatusCode::BAD_REQUEST, API_ERROR.to_owned())]).await;
    let hooks = fixture.hooks_fail_open();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("fail open");
    assert!(standard_records(&recorder).is_empty());
    assert_eq!(records(&request, "metadata").len(), 1);
    assert_eq!(contents(&response), vec!["hello".to_owned()]);
}

#[tokio::test]
async fn post_call_without_model_call_details_still_blocks() {
    let fixture = armor(vec![(StatusCode::OK, BLOCKED.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    recorder.model_call_details = None;
    let error = hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (status, _detail) = blocked(error);
    assert_eq!(status, 400);
}

#[tokio::test]
async fn post_call_does_not_mask_without_the_setting() {
    let fixture = armor(vec![(StatusCode::OK, SANITIZED_TEXT.to_owned())]).await;
    let hooks = fixture.hooks();
    let mut request = prompt_data();
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect("clean");
    assert_eq!(contents(&response), vec!["hello".to_owned()]);
}

#[tokio::test]
async fn post_call_blocks_a_flagged_response_whatever_the_start_time_is() {
    let fixture = armor(vec![(StatusCode::OK, BLOCKED.to_owned())]).await;
    let hooks = fixture.hooks_fail_open();
    let mut request = prompt_data();
    request.insert("start_time".to_owned(), text("soon"));
    let mut response = model_response(r#"{"choices":[{"message":{"content":"hello"}}]}"#);
    let mut recorder = Recorder::new(Some(GuardrailEventHooks::PostCall));
    let error = hooks
        .post_call_success(&mut request, &mut response, &mut recorder.context())
        .await
        .expect_err("blocked");
    let (status, detail) = blocked(error);
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Response blocked by Model Armor"))
    );
    let entries = standard_records(&recorder);
    assert_eq!(field(&entries[0], "start_time"), GuardrailValue::Null);
}
