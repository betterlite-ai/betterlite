use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{AgenticCallback as _, AgenticGate, AgenticOverrides};

use crate::primitives::json_value::JsonValue;

use super::support;

const CHAT: &str = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}}]}"#;
const RESPONSES: &str = r#"{"output":[{"type":"function_call","call_id":"call_1","name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}]}"#;
const ANTHROPIC: &str = r#"{"content":[{"type":"tool_use","id":"toolu_1","name":"headroom_retrieve","input":{"hash":"abcdef123456"}}]}"#;
const EXTRACTED: &str = r#"[{"id":"call_1","type":"function","name":"headroom_retrieve","arguments":{"hash":"abcdef123456"}}]"#;

async fn gate_result(response: &str) -> Result<AgenticGate, AgenticHookError> {
    let call = support::gate_call(response);
    let harness = support::harness("http://127.0.0.1:1");
    harness
        .guardrail
        .should_run_agentic_loop(&call.view())
        .await
}

async fn gate(response: &str) -> AgenticGate {
    gate_result(response).await.unwrap()
}

async fn gate_error(response: &str) -> AgenticHookError {
    gate_result(response).await.unwrap_err()
}

fn extracted(gate: &AgenticGate) -> JsonValue {
    support::json(gate.tools.get("tool_calls").expect("tool_calls").get())
}

#[test]
fn the_headroom_callback_announces_itself() {
    let harness = support::harness("http://127.0.0.1:1");
    assert_eq!(harness.guardrail.callback_name(), "headroom");
    assert_eq!(
        harness.guardrail.overrides(),
        AgenticOverrides {
            should_run: true,
            run_loop: false,
            build_plan: true,
            post_response: false,
            cleanup: false,
        }
    );
}

#[tokio::test]
async fn the_loop_runs_for_retrieve_calls_in_a_chat_response() {
    let gate = gate(CHAT).await;
    assert!(gate.run);
    assert_eq!(gate.tools.len(), 1);
    assert_eq!(extracted(&gate), support::json(EXTRACTED));
}

#[tokio::test]
async fn the_loop_runs_for_responses_function_calls() {
    let gate = gate(RESPONSES).await;
    assert!(gate.run);
    assert_eq!(extracted(&gate), support::json(EXTRACTED));
}

#[tokio::test]
async fn the_loop_runs_for_anthropic_tool_uses() {
    let gate = gate(ANTHROPIC).await;
    assert!(gate.run);
    assert_eq!(
        extracted(&gate),
        support::json(
            r#"[{"id":"toolu_1","type":"function","name":"headroom_retrieve","arguments":{"hash":"abcdef123456"}}]"#
        )
    );
}

#[tokio::test]
async fn the_loop_is_skipped_without_the_request_tool() {
    let mut call = support::gate_call(CHAT);
    call.tools = None;
    let harness = support::harness("http://127.0.0.1:1");
    let gate = harness
        .guardrail
        .should_run_agentic_loop(&call.view())
        .await
        .unwrap();
    assert!(!gate.run);
    assert!(gate.tools.is_empty());
}

#[tokio::test]
async fn the_loop_is_skipped_without_retrieve_calls() {
    let gate = gate(r#"{"choices":[{"message":{"content":"just text"}}]}"#).await;
    assert!(!gate.run);
    assert!(gate.tools.is_empty());
}

#[tokio::test]
async fn the_loop_is_skipped_for_other_tool_calls() {
    let response = r#"{"choices":[{"message":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}"#;
    let gate = gate(response).await;
    assert!(!gate.run);
}

#[tokio::test]
async fn only_retrieve_calls_are_kept() {
    let response = r#"{"choices":[{"message":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}},{"id":"call_2","type":"function","function":{"name":"headroom_retrieve","arguments":"{\"hash\":\"abcdef123456\"}"}}]}}]}"#;
    let gate = gate(response).await;
    assert_eq!(
        extracted(&gate),
        support::json(
            r#"[{"id":"call_2","type":"function","name":"headroom_retrieve","arguments":{"hash":"abcdef123456"}}]"#
        )
    );
}

#[tokio::test]
async fn the_loop_is_skipped_before_reading_the_response() {
    let mut call = support::gate_call("[1]");
    call.tools = None;
    let harness = support::harness("http://127.0.0.1:1");
    let gate = harness
        .guardrail
        .should_run_agentic_loop(&call.view())
        .await
        .unwrap();
    assert!(!gate.run);
}

#[tokio::test]
async fn an_unreadable_response_is_a_hook_error() {
    match gate_error("[1]").await {
        AgenticHookError::Failed { callback, message } => {
            assert_eq!(callback, "headroom");
            assert!(message.contains("not a chat"));
        }
    }
}
