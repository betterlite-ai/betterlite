#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;

use crate::guardrail_records::invocation::{GuardrailHookName, GuardrailInvocation};
use crate::microsoft_purview::purview_dlp_hooks::context::HookContext;
use crate::microsoft_purview::purview_dlp_hooks::error::PurviewHookError;
use crate::microsoft_purview::purview_dlp_hooks::purview_pre_call_hook::async_pre_call_hook;
use crate::microsoft_purview::purview_dlp_hooks::request_text::TOKEN_ID_ERROR;
use crate::microsoft_purview::purview_dlp_hooks::test_support::{
    Harness, Scan, Script, blocking_actions, data, fixed_clock, guardrail_at, hook_context, start,
    trusted_identity, untrusted_identity,
};
use crate::purview_dlp_evaluation::error_handling::PurviewDlpHttpError;
use crate::purview_dlp_evaluation::user_id_blocking::UNTRUSTED_IDENTITY_ERROR;

fn http_error(result: Result<(), PurviewHookError>) -> PurviewDlpHttpError {
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

#[tokio::test]
async fn blocks_a_prompt_the_policy_rejects() {
    let mock = start(Script::new(vec![blocking_actions()]));
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(
        r#"{"model":"gpt","messages":[{"role":"user","content":"secret"}]}"#,
    ));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::Completion),
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(
        detail(&error, "error").as_deref(),
        Some("Microsoft Purview DLP: Content blocked by policy")
    );
    assert_eq!(detail(&error, "activity").as_deref(), Some("uploadText"));
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "uploadText".to_owned(),
            data: "secret".to_owned(),
        }]
    );
}

#[tokio::test]
async fn returns_the_request_unchanged_after_a_clean_scan() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(
        r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}]}"#,
    ));
    let messages = harness.data.get("messages").cloned();
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::Completion),
        &mut context,
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(harness.data.get("messages"), messages.as_ref());
    assert_eq!(mock.process_calls(), 1);
}

#[tokio::test]
async fn rejects_token_id_completion_prompts_before_the_scan() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","prompt":[1,2,3]}"#));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::TextCompletion),
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(detail(&error, "error").as_deref(), Some(TOKEN_ID_ERROR));
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn scans_a_text_completion_prompt_after_normalizing_it() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","prompt":"  hello  "}"#));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::AtextCompletion),
        &mut context,
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "uploadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn routes_responses_calls_through_the_input_extractor() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(
        r#"{"model":"gpt","instructions":"be careful","input":"hi"}"#,
    ));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::Responses),
        &mut context,
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "uploadText".to_owned(),
            data: "be careful\n\nhi".to_owned(),
        }]
    );
}

#[tokio::test]
async fn scans_the_text_of_every_chat_message() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(
        r#"{"model":"gpt","messages":[{"role":"system","content":"sys"},{"role":"user","content":"hello"}]}"#,
    ));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::Acompletion),
        &mut context,
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "uploadText".to_owned(),
            data: "sys\n\nhello".to_owned(),
        }]
    );
}

#[tokio::test]
async fn skips_the_scan_when_the_request_has_no_prompt_text() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(r#"{"model":"gpt","messages":[]}"#));
    let mut context = hook_context(
        trusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::Completion),
        &mut context,
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn rejects_an_identity_the_proxy_did_not_authenticate() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(
        r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}]}"#,
    ));
    let mut context = hook_context(
        untrusted_identity(),
        &harness.cache,
        &harness.redactor,
        &harness.base,
        &harness.sink,
    );
    let result = async_pre_call_hook(
        &guardrail,
        &mut harness.data,
        Some(CallType::Completion),
        &mut context,
    )
    .await;
    let error = http_error(result);
    assert_eq!(error.status, 400);
    assert_eq!(
        detail(&error, "error").as_deref(),
        Some(UNTRUSTED_IDENTITY_ERROR)
    );
    assert_eq!(mock.process_calls(), 0);
}

#[tokio::test]
async fn records_the_scan_through_a_started_invocation() {
    let mock = start(Script::passing());
    let guardrail = guardrail_at(&mock.base);
    let mut harness = Harness::new(data(
        r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}]}"#,
    ));
    let mut invocation = GuardrailInvocation::begin(
        GuardrailHookName::PreCall,
        &harness.data,
        None,
        fixed_clock(),
    )
    .unwrap();
    let result = {
        let mut context = HookContext {
            identity: trusted_identity(),
            cache: Arc::clone(&harness.cache),
            redactor: Arc::clone(&harness.redactor),
            record_base: &harness.base,
            invocation: Some(&mut invocation),
            sink: &harness.sink,
            log_target: None,
        };
        async_pre_call_hook(
            &guardrail,
            &mut harness.data,
            Some(CallType::Completion),
            &mut context,
        )
        .await
    };
    assert!(result.is_ok());
    assert!(invocation.self_recorded);
    assert_eq!(harness.sink.records().len(), 1);
}
