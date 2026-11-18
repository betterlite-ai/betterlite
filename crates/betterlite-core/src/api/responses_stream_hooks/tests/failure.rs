use std::str::FromStr as _;

use betterlite_error::error::ErrorClass;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::failure::await_pending_logging;
use super::super::state::ResponsesStreamHookState;
use super::support::{
    Calculator, END, FailureRecorder, START, base_state, block_on_local, decode, error_event,
    failure_class, hooks, number, path, redactor, response_failed_event, timestamp,
};

fn fallback(class: &ErrorClass) -> &betterlite_error::error::details::MidStreamFallbackDetails {
    match class {
        ErrorClass::MidStreamFallback(details) => details,
        other => panic!("expected a mid-stream fallback, got {other:?}"),
    }
}

#[tokio::test]
async fn failure_callback_order() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failure = FailureRecorder::recording();
    hooks.failure = Some(std::sync::Arc::<FailureRecorder>::clone(&failure));
    let mut state = base_state();
    let mut callback = ResponsesStreamHookState::default();
    let class = failure_class();

    hooks
        .handle_failure(&mut state, &mut callback, &class)
        .await;
    assert!(state.failure_handled);
    assert_eq!(callback.pending_logging_tasks.len(), 1);
    assert!(failure.async_calls().is_empty());

    hooks
        .handle_failure(&mut state, &mut callback, &class)
        .await;
    assert_eq!(callback.pending_logging_tasks.len(), 1);

    await_pending_logging(&mut callback).await;

    assert!(callback.pending_logging_tasks.is_empty());
    assert_eq!(failure.order(), vec!["async", "sync"]);
    let asynchronous = &failure.async_calls()[0];
    let synchronous = &failure.sync_calls()[0];
    assert_eq!(asynchronous.start_time, timestamp(START));
    assert_eq!(asynchronous.end_time, timestamp(END));
    assert_eq!(asynchronous.traceback_exception, "");
    assert_eq!(asynchronous.exception, synchronous.exception);
    assert_eq!(asynchronous.start_time, synchronous.start_time);
    assert_eq!(asynchronous.end_time, synchronous.end_time);
    assert_eq!(
        asynchronous.traceback_exception,
        synchronous.traceback_exception
    );
    assert_eq!(asynchronous.exception.metadata().status_code, 400);
}

#[tokio::test]
async fn failure_callback_order_submits_the_sync_handler_after_an_async_failure() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failure = FailureRecorder::failing_async();
    hooks.failure = Some(std::sync::Arc::<FailureRecorder>::clone(&failure));
    let mut state = base_state();
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .handle_failure(&mut state, &mut callback, &failure_class())
        .await;
    await_pending_logging(&mut callback).await;

    assert_eq!(failure.order(), vec!["async", "sync"]);
    assert_eq!(failure.async_calls().len(), 1);
    assert_eq!(failure.sync_calls().len(), 1);
}

#[tokio::test]
async fn failure_callback_order_swallows_a_rejected_submission() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failure = FailureRecorder::failing_submit();
    hooks.failure = Some(std::sync::Arc::<FailureRecorder>::clone(&failure));
    let mut state = base_state();
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .handle_failure(&mut state, &mut callback, &failure_class())
        .await;
    await_pending_logging(&mut callback).await;

    assert_eq!(failure.order(), vec!["async", "sync"]);
}

#[tokio::test]
async fn failure_callback_order_marks_a_handled_failure_without_a_port() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let mut state = base_state();
    let mut callback = ResponsesStreamHookState::default();

    hooks
        .handle_failure(&mut state, &mut callback, &failure_class())
        .await;

    assert!(state.failure_handled);
    assert!(callback.pending_logging_tasks.is_empty());
}

#[test]
fn failure_callback_order_without_a_loop_runs_both_inline() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failure = FailureRecorder::recording();
    hooks.failure = Some(std::sync::Arc::<FailureRecorder>::clone(&failure));
    let mut state = base_state();
    let mut callback = ResponsesStreamHookState::default();
    let class = failure_class();

    block_on_local(async {
        hooks
            .handle_failure(&mut state, &mut callback, &class)
            .await;
    });

    assert!(callback.pending_logging_tasks.is_empty());
    assert_eq!(failure.order(), vec!["async", "sync"]);
    assert_eq!(failure.sync_calls().len(), 1);
}

#[tokio::test]
async fn failed_usage_and_error() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    let failure = FailureRecorder::recording();
    hooks.failure = Some(std::sync::Arc::<FailureRecorder>::clone(&failure));
    let mut state = base_state();
    state.completed_response = Some(response_failed_event(
        Some(r#"{"input_tokens":3,"output_tokens":4,"total_tokens":7}"#),
        Some(r#"{"message":"boom","type":"invalid_request_error","code":"bad"}"#),
    ));
    let mut callback = ResponsesStreamHookState::default();
    let mut details = IndexMap::new();
    let calculator = Calculator::charging(Decimal::from_str("0.5").expect("a decimal"));

    hooks
        .handle_logging_failed_response(&mut state, &mut callback, &mut details, Some(&calculator))
        .await;
    await_pending_logging(&mut callback).await;

    let combined = details
        .get("combined_usage_object")
        .expect("the failed usage is recorded");
    assert_eq!(path(combined, &["prompt_tokens"]), Some(&number("3")));
    assert_eq!(path(combined, &["completion_tokens"]), Some(&number("4")));
    assert_eq!(path(combined, &["total_tokens"]), Some(&number("7")));
    assert_eq!(details.get("response_cost"), Some(&number("0.5")));
    assert!(state.failure_handled);
    let delivered = &failure.async_calls()[0];
    assert!(matches!(delivered.exception, ErrorClass::BadRequest(_)));
    let ErrorClass::BadRequest(details) = &delivered.exception else {
        unreachable!("the class was matched above");
    };
    assert!(details.message.contains("boom"));
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[tokio::test]
async fn failed_usage_and_error_defaults_the_cost_to_zero() {
    let redactor = redactor();
    let usage = Some(r#"{"input_tokens":3,"output_tokens":4,"total_tokens":7}"#);
    let event = Some(r#"{"message":"boom","type":"invalid_request_error","code":"bad"}"#);
    for calculator in [Calculator::missing(), Calculator::failing()] {
        let mut hooks = hooks(&redactor);
        hooks.failure = Some(FailureRecorder::recording());
        let mut state = base_state();
        state.completed_response = Some(response_failed_event(usage, event));
        let mut callback = ResponsesStreamHookState::default();
        let mut details = IndexMap::new();

        hooks
            .handle_logging_failed_response(
                &mut state,
                &mut callback,
                &mut details,
                Some(&calculator),
            )
            .await;

        assert_eq!(details.get("response_cost"), Some(&number("0")));
    }
}

#[tokio::test]
async fn failed_usage_and_error_skips_billing_without_usage_or_a_calculator() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    hooks.failure = Some(FailureRecorder::recording());
    let mut state = base_state();
    state.completed_response = Some(response_failed_event(
        None,
        Some(r#"{"message":"boom","type":"invalid_request_error","code":"bad"}"#),
    ));
    let mut details = IndexMap::new();

    hooks
        .handle_logging_failed_response::<Calculator>(
            &mut state,
            &mut ResponsesStreamHookState::default(),
            &mut details,
            None,
        )
        .await;

    assert!(!details.contains_key("combined_usage_object"));
    assert!(!details.contains_key("response_cost"));
    assert!(state.failure_handled);
}

#[tokio::test]
async fn failed_usage_and_error_skips_billing_for_an_untyped_response() {
    let redactor = redactor();
    let mut hooks = hooks(&redactor);
    hooks.failure = Some(FailureRecorder::recording());
    let mut state = base_state();
    state.completed_response = Some(error_event("boom", "server_error", "server_error"));
    let mut details = IndexMap::new();

    hooks
        .handle_logging_failed_response::<Calculator>(
            &mut state,
            &mut ResponsesStreamHookState::default(),
            &mut details,
            None,
        )
        .await;

    assert!(!details.contains_key("combined_usage_object"));
    assert!(!details.contains_key("response_cost"));
    assert!(state.failure_handled);
}

#[test]
fn failed_usage_and_error_raises_the_mapped_error_for_an_ineligible_status() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    for (error_type, code, status) in [
        ("invalid_request_error", "invalid_request_error", 400),
        ("authentication_error", "401", 401),
        ("not_found_error", "404", 404),
    ] {
        let event = error_event("boom", error_type, code);
        let raised = hooks
            .maybe_raise_for_error_event(&state, &event)
            .expect_err("a 4xx error is raised directly");
        assert!(!matches!(
            raised.exception,
            ErrorClass::MidStreamFallback(_)
        ));
        assert_eq!(raised.exception.metadata().status_code, status);
    }
}

#[test]
fn failed_usage_and_error_wraps_an_eligible_failure_in_an_error_event() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let mut state = base_state();
    state.generated_content = "text only".to_owned();
    state.generated_tool_arguments = "tool args".to_owned();
    let event = error_event("boom", "server_error", "server_error");

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a 5xx error falls back");
    let details = fallback(&raised.exception);

    assert_eq!(details.original_status.0, 500);
    assert_eq!(details.generated_content, "text only");
    assert!(details.is_pre_first_chunk);
    assert!(details.context.message.contains("boom"));
}

#[test]
fn failed_usage_and_error_uses_the_yielded_chunk_not_the_output_flag() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let mut state = base_state();
    state.yielded_first_chunk = true;
    state.output_started = false;
    let event = error_event("boom", "server_error", "server_error");

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a 5xx error falls back");
    let details = fallback(&raised.exception);

    assert!(!details.is_pre_first_chunk);
}

#[test]
fn failed_usage_and_error_falls_back_for_a_content_policy_violation() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = error_event(
        "blocked",
        "invalid_request_error",
        "content_policy_violation",
    );

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a content policy violation falls back");
    assert_eq!(fallback(&raised.exception).original_status.0, 400);
}

#[test]
fn failed_usage_and_error_maps_a_failed_response_event() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = response_failed_event(
        None,
        Some(r#"{"message":"boom","type":"server_error","code":"server_error"}"#),
    );

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a failed response falls back");
    assert_eq!(fallback(&raised.exception).original_status.0, 500);
}

#[test]
fn failed_usage_and_error_maps_a_generic_error_object() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = decode(
        r#"{"type":"error","error":{"message":"boom","type":"invalid_request_error","code":"invalid_request_error"}}"#,
    );

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a generic error is mapped");
    assert_eq!(raised.exception.metadata().status_code, 400);
    let ErrorClass::BadRequest(details) = &raised.exception else {
        panic!("expected a bad request, got {raised:?}");
    };
    assert!(details.message.contains("boom"));
}

#[test]
fn failed_usage_and_error_defaults_a_generic_error_without_a_body() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = decode(r#"{"type":"error"}"#);

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a generic error without a body falls back");
    assert_eq!(fallback(&raised.exception).original_status.0, 500);
    assert_eq!(
        fallback(&raised.exception).context.message,
        "InternalServerError: OpenAIException - Response API in-stream error"
    );
}

#[test]
fn failed_usage_and_error_defaults_a_generic_failed_response() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = decode(r#"{"type":"response.failed"}"#);

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a generic failed response falls back");
    assert_eq!(fallback(&raised.exception).original_status.0, 500);
}

#[test]
fn failed_usage_and_error_passes_every_other_event() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    for text in [
        r#"{"type":"response.created","response":{"id":"resp_1","created_at":1700000000,"model":"gpt-4o","output":[]}}"#,
        r#"{"type":"response.queued","id":"resp_1"}"#,
    ] {
        let event = decode(text);
        assert!(hooks.maybe_raise_for_error_event(&state, &event).is_ok());
    }
}

#[test]
fn failed_usage_and_error_keeps_the_model_and_provider() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = error_event("boom", "invalid_request_error", "invalid_request_error");

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a 4xx error is raised");
    let metadata = raised.exception.metadata();
    assert_eq!(metadata.model.as_deref(), Some("gpt-4o"));
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
}

#[test]
fn failed_usage_and_error_reads_a_text_only_generated_content() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let mut state = base_state();
    state.generated_content = "text".to_owned();
    state.generated_tool_arguments = "args".to_owned();
    let event = error_event("boom", "rate_limit_exceeded", "rate_limit_exceeded");

    let raised = hooks
        .maybe_raise_for_error_event(&state, &event)
        .expect_err("a rate limit falls back");
    let details = fallback(&raised.exception);
    assert_eq!(details.original_status.0, 429);
    assert_eq!(details.generated_content, "text");
}

#[test]
fn failed_usage_and_error_ignores_a_json_error_of_another_event() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = decode(r#"{"type":"response.output_text.delta"}"#);
    assert!(hooks.maybe_raise_for_error_event(&state, &event).is_ok());
}

#[test]
fn fallback_keeps_original_exception() {
    let redactor = redactor();
    let hooks = hooks(&redactor);
    let state = base_state();
    let event = error_event("boom", "server_error", "server_error");
    let failure = hooks
        .maybe_raise_for_error_event(&state, &event)
        .unwrap_err();
    let original = failure.original_exception.as_ref().unwrap();
    assert!(matches!(original, ErrorClass::InternalServer(_)));
    assert_eq!(
        original.metadata().status_code,
        failure.exception.metadata().status_code
    );
}

#[test]
fn async_failure_diagnostics_keep_the_task_name() {
    let ((), logs) = super::support::capture(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let redactor = redactor();
            let mut hooks = hooks(&redactor);
            let failure = FailureRecorder::failing_async();
            hooks.failure = Some(std::sync::Arc::<FailureRecorder>::clone(&failure));
            let mut state = base_state();
            let mut callback = ResponsesStreamHookState::default();
            hooks
                .handle_failure(&mut state, &mut callback, &failure_class())
                .await;
            await_pending_logging(&mut callback).await;
            assert_eq!(failure.order(), vec!["async", "sync"]);
        });
    });
    assert!(
        logs.messages()
            .iter()
            .any(|line| line.contains("Responses stream failure logging failed: async failed"))
    );
}
