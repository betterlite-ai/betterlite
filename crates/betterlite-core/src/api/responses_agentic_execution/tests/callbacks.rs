use std::sync::{Arc, Mutex};

use betterlite_context::agentic_loop_plan::JsonMap;

use super::super::AgenticResult;
use super::super::error::AgenticExecutionError;
use super::support::{
    NEXT_BODY, RecordingCallback, ScriptedExecutor, callback_list, continuing_script, dispatch,
    fixture, legacy_script, null_legacy_script, order_log, recording, refused_tools,
};
use super::{
    assert_log, continue_messages, dispatch_error, plan_callback, raw_text, response_text,
};
use crate::api::chat_agentic_loop::tests::support::{
    Script, capture, gate, json_map, overrides, patch,
};

pub(super) async fn plan_hooks_in_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let skipped = recording("skipped", Script::default(), &order);
    let throwing = recording(
        "a",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate_error("boom"),
        &order,
    );
    let quiet = recording(
        "c",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, JsonMap::new())),
        &order,
    );
    let tools = refused_tools("web_search", "call-ws");
    let follower = plan_callback("b", &order, &tools, patch(Some(continue_messages())));
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&skipped, &throwing, &quiet, &follower]),
        &executor,
    )
    .await
    .expect("the dispatcher runs");
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in async_should_run_agentic_loop [call_id=call-1 model=gpt-4o]: boom",
    );
    assert_hook_calls(&order, &throwing, &follower, &tools);
    let answer = outcome.expect("the plan path answers");
    assert_eq!(response_text(answer), NEXT_BODY);
}

fn assert_hook_calls(
    order: &Arc<Mutex<Vec<String>>>,
    throwing: &RecordingCallback,
    follower: &RecordingCallback,
    tools: &JsonMap,
) {
    let expected = [
        "a:gate",
        "c:gate",
        "b:gate",
        "b:plan",
        "b:post",
        "b:cleanup",
    ];
    assert_eq!(order_log(order), expected);
    let gate_records = throwing.records();
    let gate_call = &gate_records.gate[0];
    assert_eq!(
        raw_text(&gate_call.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("\"responses\"")
    );
    assert_eq!(raw_text(&gate_call.kwargs, "custom_llm_provider"), None);
    let plan_records = follower.records();
    let plan_call = &plan_records.plan[0];
    assert_eq!(
        raw_text(&plan_call.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("\"responses\"")
    );
    assert_eq!(
        raw_text(&plan_call.kwargs, "custom_llm_provider").as_deref(),
        Some("\"openai\"")
    );
    assert_eq!(&plan_call.tools, tools);
}

async fn legacy_dispatch(script: Script) -> (Option<AgenticResult>, Vec<String>) {
    let order = Arc::new(Mutex::new(Vec::new()));
    let first = recording("le", script, &order);
    let never = recording("never", Script::default(), &order);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let outcome = dispatch(&fixture(), &callback_list(&[&first, &never]), &executor)
        .await
        .expect("the dispatcher runs");
    (outcome, order_log(&order))
}

pub(super) async fn legacy_answers() {
    let (outcome, order) = legacy_dispatch(legacy_script(r#"{"legacy":true}"#)).await;
    let answer = outcome.expect("the legacy answer");
    assert_eq!(response_text(answer), r#"{"legacy":true}"#);
    assert_eq!(order, ["le:gate", "le:legacy"]);
}

pub(super) async fn null_legacy_is_no_result() {
    let (outcome, order) = legacy_dispatch(null_legacy_script()).await;
    assert!(outcome.is_none());
    assert_eq!(order, ["le:gate", "le:legacy"]);
}

pub(super) async fn plan_error_moves_on() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let broken = recording(
        "br",
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, JsonMap::new()))
            .with_plan_error("plan broke"),
        &order,
    );
    let answering = legacy("le", &order, r#"{"legacy":true}"#);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let (log, _guard) = capture();
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&broken, &answering]),
        &executor,
    )
    .await
    .expect("a callback failure is logged, not raised");
    assert_log(
        &log,
        "LiteLLM.AgenticHookError: Exception in agentic completion hooks [call_id=call-1 model=gpt-4o]: plan broke",
    );
    assert_eq!(
        order_log(&order),
        ["br:gate", "br:plan", "le:gate", "le:legacy"]
    );
    let answer = outcome.expect("the next callback answers");
    assert_eq!(response_text(answer), r#"{"legacy":true}"#);
}

pub(super) async fn run_loop_false_moves_on() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let continuing = recording(
        "co",
        continuing_script(&refused_tools("web_search", "call-ws")),
        &order,
    );
    let answering = legacy("le", &order, r#"{"legacy":true}"#);
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let outcome = dispatch(
        &fixture(),
        &callback_list(&[&continuing, &answering]),
        &executor,
    )
    .await
    .expect("the dispatcher runs");
    assert_eq!(executor.call_count(), 0);
    assert_eq!(
        order_log(&order),
        ["co:gate", "co:plan", "le:gate", "le:legacy"]
    );
    let answer = outcome.expect("the next callback answers");
    assert_eq!(response_text(answer), r#"{"legacy":true}"#);
}

pub(super) async fn no_callbacks_is_no_result() {
    let executor = ScriptedExecutor::responding(NEXT_BODY);
    let outcome = dispatch(&fixture(), &[], &executor)
        .await
        .expect("no callbacks is no result");
    assert!(outcome.is_none());
}

pub(super) async fn optional_tools_reach_the_gate() {
    let mut tooled = fixture();
    tooled.optional_params = json_map(&[(
        "tools",
        r#"[{"type":"function","function":{"name":"lookup"}}]"#,
    )]);
    let order = Arc::new(Mutex::new(Vec::new()));
    let quiet = recording(
        "q",
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, JsonMap::new())),
        &order,
    );
    let outcome = dispatch(
        &tooled,
        &callback_list(&[&quiet]),
        &ScriptedExecutor::responding(NEXT_BODY),
    )
    .await
    .expect("the optional tools parse");
    assert!(outcome.is_none());
    let records = quiet.records();
    let gate_call = &records.gate[0];
    let tools = gate_call.tools.as_ref().expect("the gate sees the tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(
        tools[0].0.get(),
        r#"{"type":"function","function":{"name":"lookup"}}"#
    );
}

pub(super) async fn invalid_optional_tools_raise() {
    let mut broken = fixture();
    broken.optional_params = json_map(&[("tools", "\"not-a-list\"")]);
    let outcome = dispatch(&broken, &[], &ScriptedExecutor::responding(NEXT_BODY)).await;
    let error = dispatch_error(outcome, "invalid optional tools raise");
    assert!(matches!(error, AgenticExecutionError::InvalidTools { .. }));
}
