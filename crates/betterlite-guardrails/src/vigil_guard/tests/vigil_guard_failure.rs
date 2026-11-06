use betterlite_guardrails::generic_guardrail_contracts::UnreachableFallback;
use betterlite_guardrails::vigil_guard::vigil_guard_error::VigilGuardError;
use betterlite_guardrails::vigil_guard::vigil_guard_failure::handle_backend_failure;
use betterlite_guardrails::vigil_guard::vigil_guard_transient::TransportError;
use reqwest::StatusCode;

use crate::support::{failure_context, inputs, text, tool_call};

fn backend_failure() -> VigilGuardError {
    VigilGuardError::Transport {
        source: TransportError::Timeout,
    }
}

#[test]
fn fail_open_allows_the_request_unchanged() {
    let inputs = inputs(Some(vec!["a".to_owned()]), None);
    let final_texts = vec!["a".to_owned()];
    let result = handle_backend_failure(
        &failure_context(UnreachableFallback::FailOpen),
        &backend_failure(),
        &inputs,
        &final_texts,
        None,
    )
    .expect("fail_open allows the request");
    assert_eq!(result, inputs);
}

#[test]
fn fail_open_keeps_sanitized_texts_and_drops_structured_messages() {
    let mut inputs = inputs(Some(vec!["a".to_owned(), "b".to_owned()]), None);
    inputs.images = Some(vec![text("image")]);
    inputs.tools = Some(vec![text("tool")]);
    inputs.structured_messages = Some(vec![text("stale")]);
    let final_texts = vec!["sanitized".to_owned(), "b".to_owned()];
    let result = handle_backend_failure(
        &failure_context(UnreachableFallback::FailOpen),
        &backend_failure(),
        &inputs,
        &final_texts,
        None,
    )
    .expect("fail_open allows the request");
    assert_eq!(result.texts, Some(final_texts));
    assert_eq!(result.images, inputs.images);
    assert_eq!(result.tools, inputs.tools);
    assert_eq!(result.structured_messages, None);
    assert_eq!(result.model, None);
}

#[test]
fn fail_open_keeps_a_changed_tool_call_list() {
    let inputs = inputs(Some(vec!["a".to_owned()]), Some(vec![tool_call("old")]));
    let final_calls = vec![tool_call("new")];
    let final_texts = vec!["a".to_owned()];
    let result = handle_backend_failure(
        &failure_context(UnreachableFallback::FailOpen),
        &backend_failure(),
        &inputs,
        &final_texts,
        Some(&final_calls),
    )
    .expect("fail_open allows the request");
    assert_eq!(result.tool_calls, Some(final_calls));
}

#[test]
fn fail_closed_blocks_the_request() {
    let inputs = inputs(Some(vec!["a".to_owned()]), None);
    let final_texts = vec!["a".to_owned()];
    let error = handle_backend_failure(
        &failure_context(UnreachableFallback::FailClosed),
        &backend_failure(),
        &inputs,
        &final_texts,
        None,
    )
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert!(matches!(
        error,
        VigilGuardError::Blocked { reason } if reason.contains("fail_closed")
    ));
}
