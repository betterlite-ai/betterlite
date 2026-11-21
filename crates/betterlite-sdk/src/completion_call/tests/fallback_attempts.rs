use betterlite_models::decisions::Json;

use super::support::{FakeIds, RecordingTarget, args_from, dig, json, text_of};
use super::{USER, body_text};
use crate::completion_call::error::CompletionCallError;
use crate::completion_call::fallbacks::run_fallbacks;
use crate::completion_request::call::FallbackCall;

#[tokio::test]
async fn fallback_tries_the_original_each_entry_and_keeps_overrides() {
    let target = RecordingTarget::failing_then_ok(2);
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![
            json(r#""gpt-3.5-turbo""#),
            json(r#"{"model":"claude","api_base":"https://claude.test"}"#),
        ],
        args,
    };
    let outcome = run_fallbacks(&target, call, None, &FakeIds).await.unwrap();
    let calls = target.call_map();
    assert_eq!(calls.len(), 3);
    assert_eq!(text_of(calls[0].get("model")), Some("gpt-4o"));
    assert_eq!(text_of(calls[1].get("model")), Some("gpt-3.5-turbo"));
    assert_eq!(text_of(calls[2].get("model")), Some("claude"));
    assert_eq!(
        text_of(calls[2].get("api_base")),
        Some("https://claude.test")
    );
    assert_eq!(
        text_of(calls[0].get("litellm_call_id")),
        text_of(calls[2].get("litellm_call_id"))
    );
    assert!(text_of(calls[0].get("litellm_call_id")).is_some());
    let Some(answer) = body_text(&outcome) else {
        panic!("the third attempt answers");
    };
    let attempt = dig(
        answer,
        &[
            "_hidden_params",
            "additional_headers",
            "x-litellm-attempted-fallbacks",
        ],
    );
    assert!(matches!(attempt, Some(Json::Number(number)) if number.as_u64() == Some(2)));
}

#[tokio::test]
async fn fallback_dict_without_a_model_keeps_the_original() {
    let target = RecordingTarget::failing_then_ok(1);
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![json(r#"{"temperature":0.2}"#)],
        args,
    };
    run_fallbacks(&target, call, None, &FakeIds).await.unwrap();
    let calls = target.call_map();
    assert_eq!(text_of(calls[1].get("model")), Some("gpt-4o"));
    assert!(matches!(
        calls[1].get("temperature"),
        Some(Json::Number(number)) if number.as_f64() == Some(0.2_f64)
    ));
}

#[tokio::test]
async fn fallback_internal_params_never_reach_an_attempt() {
    let target = RecordingTarget::failing_then_ok(1);
    let mut args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    args.kwargs
        .insert(String::from("skip_mcp_handler"), Json::Bool(true));
    args.kwargs
        .insert(String::from("mcp_handler_context"), Json::Bool(true));
    args.kwargs
        .insert(String::from("_skip_mcp_handler"), Json::Bool(true));
    let call = FallbackCall {
        fallbacks: vec![json(r#""gpt-3.5-turbo""#)],
        args,
    };
    run_fallbacks(&target, call, None, &FakeIds).await.unwrap();
    let attempts = target.call_map();
    assert_eq!(attempts.len(), 2);
    assert!(!attempts[0].contains_key("skip_mcp_handler"));
    assert!(!attempts[0].contains_key("mcp_handler_context"));
    assert!(!attempts[0].contains_key("_skip_mcp_handler"));
    assert!(!attempts[0].contains_key("fallbacks"));
    assert!(!attempts[0].contains_key("acompletion"));
    assert!(!attempts[1].contains_key("skip_mcp_handler"));
    assert!(!attempts[1].contains_key("mcp_handler_context"));
    assert!(!attempts[1].contains_key("_skip_mcp_handler"));
    assert!(!attempts[1].contains_key("fallbacks"));
    assert!(!attempts[1].contains_key("acompletion"));
}

#[tokio::test]
async fn fallback_all_fail_reports_litellms_text() {
    let target = RecordingTarget::always_failing();
    let args = args_from(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let call = FallbackCall {
        fallbacks: vec![json(r#""gpt-3.5-turbo""#)],
        args,
    };
    let error = run_fallbacks(&target, call, None, &FakeIds)
        .await
        .unwrap_err();
    let CompletionCallError::AllFallbacksFailed { last } = error else {
        panic!("every attempt failed");
    };
    assert_eq!(last, "first down");
    assert_eq!(
        CompletionCallError::AllFallbacksFailed { last }.to_string(),
        "first down. All fallback attempts failed. Enable verbose logging with `litellm.set_verbose=True` for details."
    );
}
