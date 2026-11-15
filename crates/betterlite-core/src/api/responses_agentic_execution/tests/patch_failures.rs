use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::JsonMap;
use serde_json::value::RawValue;

use super::super::error::{CallbackError, PatchError};
use super::support::{
    ANTHROPIC_BODY, NEXT_BODY, PlanHarness, RecordingCallback, ScriptedExecutor, anthropic_fixture,
    callback_list, dispatch, fixture, order_log, plan_harness, plan_with_patch, recording,
};
use super::{assert_log, continue_messages, legacy, response_text};
use crate::api::chat_agentic_loop::tests::support::{
    Capture, Script, StopError, capture, gate, overrides, patch, plan,
};

pub(super) async fn missing_input_raises() {
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let harness = plan_harness(fixture(), Script::default());
    let missing = plan(Some(patch(None)), None, false, true);
    let error = harness
        .responses(&missing, &executor)
        .await
        .expect_err("a responses plan without input raises");
    assert!(matches!(
        error,
        CallbackError::Patch(PatchError::MissingResponsesInput)
    ));
    assert!(harness.callback.records().cleanup.is_empty());
    assert_eq!(executor.call_count(), 0);
    let anthropic_error = harness
        .anthropic(
            &missing,
            &ScriptedExecutor::anthropic_responding(ANTHROPIC_BODY),
        )
        .await
        .expect_err("an anthropic plan without messages raises");
    assert!(matches!(
        anthropic_error,
        CallbackError::Patch(PatchError::MissingMessages)
    ));
    assert!(harness.callback.records().cleanup.is_empty());
}

pub(super) async fn provider_failure_cleans_up() {
    let valid = plan_with_patch(patch(Some(continue_messages())));
    let harness = plan_harness(fixture(), Script::default());
    let error = harness
        .responses(&valid, &ScriptedExecutor::failing("provider exploded"))
        .await
        .expect_err("a provider failure raises");
    assert_provider_failure(error);
    let records = harness.callback.records();
    assert_eq!(records.cleanup.len(), 1);
    assert!(records.post.is_empty());
}

fn assert_provider_failure(error: CallbackError<StopError>) {
    match error {
        CallbackError::Execution(source) => assert_eq!(source.message, "provider exploded"),
        other => panic!("expected the provider failure, got {other}"),
    }
}

pub(super) async fn post_failure_keeps_answer() {
    let valid = plan_with_patch(patch(Some(continue_messages())));
    let harness = plan_harness(fixture(), Script::default().with_post_error("post broke"));
    let (log, _guard) = capture();
    let response = harness
        .responses(&valid, &ScriptedExecutor::responding(NEXT_BODY))
        .await
        .expect("a post failure keeps the provider answer");
    assert_eq!(response.get(), NEXT_BODY);
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_post_agentic_loop_response_hook [call_id=call-1 model=gpt-4o]: post broke",
    );
    assert_eq!(harness.callback.records().cleanup.len(), 1);
}

fn cleanup_harness() -> PlanHarness {
    let mut harness = plan_harness(
        fixture(),
        Script::default().with_cleanup_error("cleanup broke"),
    );
    harness.state.logging = None;
    harness
}

async fn cleanup_run(
    executor: &ScriptedExecutor,
) -> (Result<Box<RawValue>, CallbackError<StopError>>, Capture) {
    let valid = plan_with_patch(patch(Some(continue_messages())));
    let harness = cleanup_harness();
    let (log, _guard) = capture();
    let outcome = harness.responses(&valid, executor).await;
    (outcome, log)
}

pub(super) async fn cleanup_failures_swallowed() {
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let (outcome, log) = cleanup_run(&executor).await;
    let response = outcome.expect("a cleanup failure keeps the provider answer");
    assert_eq!(response.get(), NEXT_BODY);
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_agentic_loop_cleanup_hook [call_id=unknown model=gpt-4o]: cleanup broke",
    );
    let failing = ScriptedExecutor::failing("provider exploded");
    let (failed, failed_log) = cleanup_run(&failing).await;
    let error = failed.expect_err("cleanup never masks the provider failure");
    assert_provider_failure(error);
    assert_log(
        &failed_log,
        "async_agentic_loop_cleanup_hook [call_id=unknown model=gpt-4o]: cleanup broke",
    );
}

fn broken_callback(order: &Arc<Mutex<Vec<String>>>) -> Arc<RecordingCallback> {
    recording(
        "br",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, JsonMap::new()))
            .with_plan(plan(Some(patch(None)), None, false, true)),
        order,
    )
}

pub(super) async fn dispatcher_plan_failure_moves_on() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let broken = broken_callback(&order);
    let answering = legacy("le", &order, r#"{"legacy":true}"#);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&broken, &answering]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("a plan failure is logged and the next callback runs");
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=gpt-4o]: Agentic loop plan missing patched responses input",
    );
    let answer = outcome.expect("the next callback answers");
    assert_eq!(response_text(answer), r#"{"legacy":true}"#);
}

pub(super) async fn anthropic_plan_failure_logged() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let broken = broken_callback(&order);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &anthropic_fixture(),
        &callback_list(&[&broken]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("an anthropic plan failure is logged");
    assert!(outcome.is_none());
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=claude-3-5-sonnet]: Agentic loop plan missing patched messages",
    );
}
