#![cfg(test)]

use std::sync::Arc;

use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage};
use serde_json::value::RawValue;

use super::maybe_run_chat_agentic_loop;
use crate::api::chat_agentic_loop::LoggingHandle;
use crate::api::chat_agentic_loop::error::AgenticLoopError;
use crate::api::chat_agentic_loop::safety::SafetyError;
use crate::api::chat_agentic_loop::tests::support::{
    CallbackRecords, FakeContinuation, Fixture, RecordedCall, RecordingPort, Script,
    ScriptedCallback, callbacks, capture, first_call, fixture, gate, json_map, loop_error,
    messages, optional_text, overrides, patch, plan, raw, raw_text,
};
use crate::api::chat_agentic_loop::{ChatLoopResult, LoopPorts};

const TOOL: &str = r#"{"type":"function","function":{"name":"get_weather"}}"#;

struct Harness {
    continuation: FakeContinuation,
    port: RecordingPort,
}

fn harness() -> Harness {
    Harness {
        continuation: FakeContinuation::responding(ChatLoopResult::Response(raw("{}"))),
        port: RecordingPort::default(),
    }
}

impl Harness {
    async fn run(
        &self,
        fixture: &Fixture,
        handlers: &[Arc<dyn AgenticCallback>],
    ) -> Result<Option<ChatLoopResult>, AgenticLoopError> {
        let ports = LoopPorts {
            continuation: &self.continuation,
            fake_stream: &self.port,
        };
        maybe_run_chat_agentic_loop(&fixture.call(), handlers, &ports).await
    }
}

fn legacy_callback(result: Option<Box<RawValue>>) -> Arc<ScriptedCallback> {
    Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(true, json_map(&[])))
            .with_legacy(result),
    ))
}

fn plan_callback(plan: AgenticLoopPlan) -> Arc<ScriptedCallback> {
    Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, json_map(&[])))
            .with_plan(plan),
    ))
}

#[tokio::test]
async fn callbacks_without_the_gate_override_are_skipped() {
    let fixture = fixture();
    let callback = Arc::new(ScriptedCallback::new(Script::default()));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(callback.records().gate.is_empty());
    assert_eq!(harness.continuation.call_count(), 0);
}

#[tokio::test]
async fn a_gate_error_is_logged_and_the_next_callback_runs() {
    let fixture = fixture();
    let failing = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate_error("gate boom"),
    ));
    let next = legacy_callback(Some(raw(r#"{"legacy":true}"#)));
    let harness = harness();
    let (logs, _guard) = capture();
    let result = harness
        .run(&fixture, &callbacks(&[&failing, &next]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"legacy":true}"#);
    assert_eq!(
        logs.messages(),
        [
            "LiteLLM.AgenticHookError: Exception in chat completion agentic gate: callback test failed: gate boom"
        ]
    );
    assert_eq!(next.records().legacy.len(), 1);
    assert!(failing.records().legacy.is_empty());
}

#[tokio::test]
async fn a_false_gate_moves_to_the_next_callback() {
    let fixture = fixture();
    let skipping = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(false, json_map(&[]))),
    ));
    let next = legacy_callback(Some(raw(r#"{"second":true}"#)));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&skipping, &next]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"second":true}"#);
    assert_eq!(skipping.records().gate.len(), 1);
    assert!(skipping.records().legacy.is_empty());
    assert_eq!(next.records().legacy.len(), 1);
}

#[tokio::test]
async fn the_gate_receives_the_tools_stream_provider_and_hook_kwargs() {
    let mut fixture = fixture();
    fixture.stream = true;
    fixture.optional_params = json_map(&[("tools", &format!("[{TOOL}]"))]);
    fixture.kwargs = json_map(&[("user", "\"u-1\"")]);
    let callback = Arc::new(ScriptedCallback::new(
        Script::default().with_overrides(overrides(true, false, false)),
    ));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert!(result.is_none());
    let records = callback.records();
    let gate = &records.gate[0];
    assert_eq!(gate.model, "gpt-4o");
    assert!(gate.stream);
    assert_eq!(gate.provider, "openai");
    let tools = gate.tools.as_ref().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].0.get(), TOOL);
    assert_eq!(raw_text(&gate.kwargs, "user").as_deref(), Some("\"u-1\""));
    assert_eq!(
        raw_text(&gate.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("\"chat_completions\"")
    );
    assert_eq!(
        raw_text(&gate.kwargs, "custom_llm_provider").as_deref(),
        Some("\"openai\"")
    );
    assert!(raw_text(&fixture.kwargs, "_agentic_loop_api_surface").is_none());
}

#[tokio::test]
async fn a_repeated_fingerprint_ends_the_call_without_a_hook_log() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[(
        "_agentic_loop_fingerprints",
        r#"["{\"call-1\":{\"name\":\"search\"}}"]"#,
    )]);
    let callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, json_map(&[("call-1", r#"{"name":"search"}"#)]))),
    ));
    let harness = harness();
    let (logs, _guard) = capture();
    let error = loop_error(harness.run(&fixture, &callbacks(&[&callback])).await);
    assert!(matches!(
        &error,
        AgenticLoopError::Safety(SafetyError::RepeatedFingerprint)
    ));
    assert!(logs.messages().is_empty());
    assert!(callback.records().plan.is_empty());
    assert_eq!(harness.continuation.call_count(), 0);
}

#[tokio::test]
async fn the_depth_cap_error_ends_the_call() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[("_agentic_loop_depth", "3")]);
    let callback = plan_callback(plan(None, None, false, true));
    let harness = harness();
    let error = loop_error(harness.run(&fixture, &callbacks(&[&callback])).await);
    assert!(matches!(
        &error,
        AgenticLoopError::Safety(SafetyError::MaxLoopsExceeded { .. })
    ));
    assert_eq!(
        error.to_string(),
        "Exceeded max_agentic_loops=3 for model=gpt-4o"
    );
}

fn assert_legacy_call(records: &CallbackRecords, state: &Fixture) {
    assert!(records.plan.is_empty());
    let legacy = &records.legacy[0];
    assert_eq!(legacy.model, "gpt-4o");
    assert_eq!(legacy.response, state.response.get());
    assert_eq!(
        legacy.messages,
        [r#"{"role":"user","content":"what is the weather"}"#.to_owned()]
    );
    assert!(!legacy.stream);
    assert_eq!(legacy.log_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        raw_text(&legacy.tools, "call-1").as_deref(),
        Some(r#"{"name":"search"}"#)
    );
    assert_eq!(
        raw_text(&legacy.kwargs, "_agentic_loop_api_surface").as_deref(),
        Some("\"chat_completions\"")
    );
    assert_eq!(
        legacy
            .optional_params
            .get("temperature")
            .map(|value| value.get()),
        Some("0.2")
    );
}

#[tokio::test]
async fn a_legacy_callback_returns_its_result_as_is() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[("_code_interpreter_interception_converted_stream", "true")]);
    fixture.optional_params = json_map(&[("temperature", "0.2")]);
    let callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(true, json_map(&[("call-1", r#"{"name":"search"}"#)])))
            .with_legacy(Some(raw(r#"{"legacy":true}"#))),
    ));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"legacy":true}"#);
    let records = callback.records();
    assert_legacy_call(&records, &fixture);
    assert_eq!(harness.continuation.call_count(), 0);
    assert!(harness.port.records().is_empty());
}

#[tokio::test]
async fn a_legacy_none_ends_the_call_without_the_final_wrap() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[("_code_interpreter_interception_converted_stream", "true")]);
    let callback = legacy_callback(None);
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(harness.port.records().is_empty());
}

#[tokio::test]
async fn a_present_response_override_wins_before_the_followup() {
    let fixture = fixture();
    let callback = plan_callback(plan(None, Some(raw("null")), false, true));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), "null");
    assert_eq!(harness.continuation.call_count(), 0);
}

#[tokio::test]
async fn a_terminating_plan_returns_the_original_response() {
    let fixture = fixture();
    let callback = plan_callback(plan(None, None, true, true));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), fixture.response.get());
    assert_eq!(harness.continuation.call_count(), 0);
}

#[tokio::test]
async fn a_plan_that_stops_moves_to_the_next_callback() {
    let mut fixture = fixture();
    fixture.optional_params = json_map(&[("temperature", "0.2")]);
    let stopping = plan_callback(plan(None, None, false, false));
    let next = legacy_callback(Some(raw(r#"{"second":true}"#)));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&stopping, &next]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"second":true}"#);
    let records = stopping.records();
    assert_eq!(records.plan.len(), 1);
    assert_eq!(records.plan[0].response, fixture.response.get());
    assert_eq!(
        records.plan[0].messages,
        [r#"{"role":"user","content":"what is the weather"}"#.to_owned()]
    );
    assert_eq!(
        records.plan[0]
            .optional_params
            .get("temperature")
            .map(|value| value.get()),
        Some("0.2")
    );
    assert!(records.legacy.is_empty());
    assert_eq!(harness.continuation.call_count(), 0);
}

#[tokio::test]
async fn a_stopped_plan_with_a_patch_does_not_run_the_followup() {
    let fixture = fixture();
    let stopping = plan_callback(plan(Some(patch(Some(messages()))), None, false, false));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&stopping]))
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(harness.continuation.call_count(), 0);
    assert!(stopping.records().cleanup.is_empty());
}

fn assert_running_plan_records(call: &RecordedCall, records: &CallbackRecords) {
    assert_eq!(call.model, "openai/gpt-4o");
    assert_eq!(
        call.messages[0].0.get(),
        r#"{"role":"user","content":"patched"}"#
    );
    assert_eq!(
        raw_text(&call.kwargs, "_agentic_loop_depth").as_deref(),
        Some("1")
    );
    assert_eq!(
        raw_text(&call.kwargs, "_agentic_loop_fingerprints").as_deref(),
        Some(r#"["{\"call-1\":{\"name\":\"search\"}}"]"#)
    );
    assert_eq!(
        records.plan[0]
            .optional_params
            .get("temperature")
            .map(|value| value.get()),
        Some("0.2")
    );
    let cleanup = &records.cleanup[0];
    assert_eq!(
        raw_text(&cleanup.kwargs, "user").as_deref(),
        Some("\"u-1\"")
    );
    assert!(cleanup.kwargs.get("_agentic_loop_api_surface").is_none());
}

#[tokio::test]
async fn a_running_plan_calls_the_continuation_and_the_cleanup() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[("user", "\"u-1\"")]);
    fixture.optional_params = json_map(&[("temperature", "0.2")]);
    let patched = vec![FollowupMessage(raw(
        r#"{"role":"user","content":"patched"}"#,
    ))];
    let callback = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, json_map(&[("call-1", r#"{"name":"search"}"#)])))
            .with_plan(plan(Some(patch(Some(patched))), None, false, true)),
    ));
    let harness = Harness {
        continuation: FakeContinuation::responding(ChatLoopResult::Response(raw(
            r#"{"ran":true}"#,
        ))),
        port: RecordingPort::default(),
    };
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"ran":true}"#);
    let call = first_call(&harness.continuation);
    let records = callback.records();
    assert_running_plan_records(&call, &records);
}

#[tokio::test]
async fn missing_patched_messages_are_logged_and_the_next_callback_runs() {
    let fixture = fixture();
    let missing = plan_callback(plan(Some(patch(None)), None, false, true));
    let next = legacy_callback(Some(raw(r#"{"second":true}"#)));
    let harness = harness();
    let (logs, _guard) = capture();
    let result = harness
        .run(&fixture, &callbacks(&[&missing, &next]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"second":true}"#);
    assert_eq!(
        logs.messages(),
        [
            "LiteLLM.AgenticHookError: Exception in chat completion agentic hooks: Agentic loop plan missing patched messages"
        ]
    );
    assert!(missing.records().cleanup.is_empty());
    assert_eq!(harness.continuation.call_count(), 0);
}

#[tokio::test]
async fn a_plan_builder_error_is_logged_and_the_next_callback_runs() {
    let fixture = fixture();
    let failing = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, true, false))
            .with_gate(gate(true, json_map(&[])))
            .with_plan_error("plan boom"),
    ));
    let next = legacy_callback(Some(raw(r#"{"second":true}"#)));
    let harness = harness();
    let (logs, _guard) = capture();
    let result = harness
        .run(&fixture, &callbacks(&[&failing, &next]))
        .await
        .unwrap();
    assert_eq!(optional_text(result), r#"{"second":true}"#);
    assert_eq!(
        logs.messages(),
        [
            "LiteLLM.AgenticHookError: Exception in chat completion agentic hooks: callback test failed: plan boom"
        ]
    );
    assert!(failing.records().legacy.is_empty());
}

#[tokio::test]
async fn a_legacy_error_is_logged_and_the_loop_ends() {
    let fixture = fixture();
    let failing = Arc::new(ScriptedCallback::new(
        Script::default()
            .with_overrides(overrides(true, false, false))
            .with_gate(gate(true, json_map(&[])))
            .with_legacy_error("legacy boom"),
    ));
    let harness = harness();
    let (logs, _guard) = capture();
    let result = harness
        .run(&fixture, &callbacks(&[&failing]))
        .await
        .unwrap();
    assert!(result.is_none());
    assert_eq!(
        logs.messages(),
        [
            "LiteLLM.AgenticHookError: Exception in chat completion agentic hooks: callback test failed: legacy boom"
        ]
    );
    assert!(failing.records().plan.is_empty());
}

#[tokio::test]
async fn invalid_tools_fail_before_the_gate() {
    let mut fixture = fixture();
    fixture.optional_params = json_map(&[("tools", r#"{"a":1}"#)]);
    let callback = Arc::new(ScriptedCallback::new(
        Script::default().with_overrides(overrides(true, false, false)),
    ));
    let harness = harness();
    let error = loop_error(harness.run(&fixture, &callbacks(&[&callback])).await);
    assert!(matches!(&error, AgenticLoopError::InvalidTools { .. }));
    assert_eq!(error.to_string(), "optional_params.tools must be a list");
    assert!(callback.records().gate.is_empty());
}

#[tokio::test]
async fn null_tools_reach_the_gate_as_an_empty_list() {
    let mut fixture = fixture();
    fixture.optional_params = json_map(&[("tools", "null")]);
    let callback = Arc::new(ScriptedCallback::new(
        Script::default().with_overrides(overrides(true, false, false)),
    ));
    let harness = harness();
    let result = harness
        .run(&fixture, &callbacks(&[&callback]))
        .await
        .unwrap();
    assert!(result.is_none());
    let records = callback.records();
    let tools = records.gate[0].tools.as_ref().unwrap();
    assert!(tools.is_empty());
}

#[tokio::test]
async fn a_settings_error_fails_before_the_gate() {
    let mut fixture = fixture();
    fixture.kwargs = json_map(&[("_agentic_loop_depth", r#""abc""#)]);
    let callback = Arc::new(ScriptedCallback::new(
        Script::default().with_overrides(overrides(true, false, false)),
    ));
    let harness = harness();
    let error = loop_error(harness.run(&fixture, &callbacks(&[&callback])).await);
    assert_eq!(
        error.to_string(),
        "_agentic_loop_depth must be a whole number: abc"
    );
    assert!(callback.records().gate.is_empty());
}

#[tokio::test]
async fn the_loop_call_has_no_logging_object_when_absent() {
    let fixture = fixture();
    let callback = legacy_callback(Some(raw(r#"{"legacy":true}"#)));
    let harness = harness();
    let ports = LoopPorts {
        continuation: &harness.continuation,
        fake_stream: &harness.port,
    };
    let result = maybe_run_chat_agentic_loop(
        &fixture.call_with(LoggingHandle::Absent),
        &callbacks(&[&callback]),
        &ports,
    )
    .await
    .unwrap();
    assert_eq!(optional_text(result), r#"{"legacy":true}"#);
    assert_eq!(callback.records().legacy[0].log_call_id, None);
}
