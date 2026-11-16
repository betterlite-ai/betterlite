#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::microsoft_purview::purview_dlp_hooks::error::PurviewHookError;
use crate::microsoft_purview::purview_dlp_hooks::purview_post_success_hook::async_post_call_success_hook;
use crate::microsoft_purview::purview_dlp_hooks::response_parts::RESPONSE_ERROR;
use crate::microsoft_purview::purview_dlp_hooks::test_support::{
    Harness, Scan, Script, blocking_actions, data, guardrail_at, hook_context, start,
    trusted_identity, untrusted_identity, value,
};
use crate::purview_dlp_evaluation::error_handling::PurviewDlpHttpError;
use crate::purview_dlp_evaluation::user_id_blocking::UNTRUSTED_IDENTITY_ERROR;

const CHAT_REQUEST: &str = r#"{"model":"gpt","messages":[{"role":"user","content":"hi"}]}"#;

fn http_error(result: Result<GuardrailValue, PurviewHookError>) -> PurviewDlpHttpError {
    match result {
        Err(PurviewHookError::Http(error)) => *error,
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

fn detail(error: &PurviewDlpHttpError, key: &str) -> Option<String> {
    match error.detail.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn chat_response(content: &str) -> String {
    format!(
        r#"{{"id":"c1","object":"chat.completion","created":1,"model":"m","choices":[{{"index":0,"message":{{"role":"assistant","content":"{content}"}}}}]}}"#
    )
}

#[tokio::test]
async fn blocks_a_response_the_policy_rejects() {
    let mock = start(Script::new(vec![blocking_actions()]));
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_success_hook(
        &guardrail,
        &mut harness.data,
        value(&chat_response("secret")),
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(detail(&error, "activity").as_deref(), Some("downloadText"));
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "secret".to_owned(),
        }]
    );
}

#[tokio::test]
async fn returns_the_response_unchanged_after_a_clean_scan() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let response = value(&chat_response("hello"));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_success_hook(
        &guardrail,
        &mut harness.data,
        response.clone(),
        &mut context,
    )
    .await;
    assert_eq!(result.unwrap(), response);
    assert_eq!(mock.process_calls(), 1);
}

#[tokio::test]
async fn scans_tool_call_arguments_with_the_message_content() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let response = value(
        r#"{"id":"c1","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"hello","tool_calls":[{"id":"t1","type":"function","function":{"name":"pay","arguments":"{\"card\":\"4111\"}"}}]}}]}"#,
    );
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    async_post_call_success_hook(&guardrail, &mut harness.data, response, &mut context)
        .await
        .unwrap();
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello\n\n---\n\n{\"card\":\"4111\"}".to_owned(),
        }]
    );
}

#[tokio::test]
async fn scans_a_text_completion_response() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","prompt":"hi"}"#));
    let response = value(r#"{"object":"text_completion","choices":[{"text":"hello"}]}"#);
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    async_post_call_success_hook(&guardrail, &mut harness.data, response, &mut context)
        .await
        .unwrap();
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn scans_a_responses_api_body_by_its_output() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","input":"hi"}"#));
    let response = value(
        r#"{"id":"r1","created_at":1,"object":"response","output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}"#,
    );
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    async_post_call_success_hook(&guardrail, &mut harness.data, response, &mut context)
        .await
        .unwrap();
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn scans_a_responses_api_body_that_omits_its_object_field() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","input":"hi"}"#));
    let response = value(
        r#"{"id":"r1","created_at":1,"output":[{"type":"message","id":"m","status":"completed","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}"#,
    );
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    async_post_call_success_hook(&guardrail, &mut harness.data, response, &mut context)
        .await
        .unwrap();
    assert_eq!(mock.scanned().len(), 1);
    assert_eq!(mock.scanned()[0].data, "hello");
}

#[tokio::test]
async fn skips_the_scan_when_the_response_has_no_text() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let response = value(r#"{"id":"c1","object":"chat.completion","choices":[]}"#);
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let returned =
        async_post_call_success_hook(&guardrail, &mut harness.data, response, &mut context)
            .await
            .unwrap();
    assert_eq!(
        returned,
        value(r#"{"id":"c1","object":"chat.completion","choices":[]}"#)
    );
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn rejects_a_responses_body_that_does_not_parse() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","input":"hi"}"#));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_success_hook(
        &guardrail,
        &mut harness.data,
        value(r#"{"object":"response","id":"r1","created_at":1,"output":"bad"}"#),
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(detail(&error, "error").as_deref(), Some(RESPONSE_ERROR));
}

#[tokio::test]
async fn rejects_an_identity_the_proxy_did_not_authenticate() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(CHAT_REQUEST));
    let mut context = hook_context(
        untrusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_post_call_success_hook(
        &guardrail,
        &mut harness.data,
        value(&chat_response("hello")),
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(
        detail(&error, "error").as_deref(),
        Some(UNTRUSTED_IDENTITY_ERROR)
    );
    assert_eq!(mock.process_calls(), 0);
}
