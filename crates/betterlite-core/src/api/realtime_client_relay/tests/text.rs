use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::close::ClientLoopExit;

use super::harness::*;

const RESPONSE_CREATE: &str = r#"{"type":"response.create"}"#;
const CANCEL: &str = r#"{"type":"response.cancel"}"#;

fn user_text(text: &str) -> String {
    format!(
        r#"{{"type":"conversation.item.create","item":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{text}"}}]}}}}"#
    )
}

fn pre_call_guardrail() -> ScriptedGuardrail {
    ScriptedGuardrail::allowing(vec![GuardrailEventHooks::PreCall])
}

async fn run_frame(guardrail: &ScriptedGuardrail, frame: &str) -> Outcome {
    ack(Fixture::default()
        .with_guardrail(guardrail.clone())
        .with_client_message(frame))
    .await
}

#[tokio::test]
async fn client_text_and_tool_guardrails() {
    joined_input_text_is_checked().await;
    blocked_text_swallows_one_response_create().await;
    tool_output_is_checked_before_the_user_role().await;
    blocked_tool_output_is_replaced_before_cancellation().await;
    empty_tool_output_is_not_checked().await;
    malformed_and_foreign_content_fall_through().await;
    non_iterable_content_ends_the_loop().await;
}

async fn joined_input_text_is_checked() {
    let guardrail = pre_call_guardrail();
    let frame = r#"{"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":"a"},{"type":"input_audio","text":"ignored"},{"type":"input_text","text":"b"}]}}"#;
    let outcome = run_frame(&guardrail, frame).await;
    assert_eq!(guardrail.applied(), vec![vec![String::from("a b")]]);
    assert_eq!(outcome.backend.parsed(), vec![json(frame)]);
    assert_eq!(outcome.pre_calls(), vec![String::from(frame)]);
    assert_eq!(outcome.exit, ClientLoopExit::ClientDisconnected);
}

async fn blocked_text_swallows_one_response_create() {
    let guardrail = ScriptedGuardrail::blocking(vec![GuardrailEventHooks::PreCall], "block me");
    let blocked = user_text("block me");
    let outcome = ack(Fixture::default()
        .with_guardrail(guardrail)
        .with_client_message(&blocked)
        .with_client_message(RESPONSE_CREATE)
        .with_client_message(RESPONSE_CREATE))
    .await;
    let backend = outcome.backend.parsed();
    assert!(!backend.contains(&json(&blocked)));
    assert_eq!(backend[0], json(CANCEL));
    assert_eq!(backend.len(), 4);
    assert_eq!(backend[2], json(RESPONSE_CREATE));
    assert_eq!(backend[3], json(RESPONSE_CREATE));
    assert_eq!(outcome.state.setup.pending_guardrail_message, None);
    assert_eq!(outcome.client_frames().len(), 1);
    assert_eq!(
        outcome.client_frames()[0].clone(),
        json(
            r#"{"type":"error","error":{"type":"guardrail_violation","message":"blocked","code":"content_policy_violation"}}"#
        )
    );
}

async fn tool_output_is_checked_before_the_user_role() {
    let guardrail = pre_call_guardrail();
    let frame = r#"{"type":"conversation.item.create","item":{"type":"function_call_output","call_id":"call-1","output":{"value":"obj"},"role":"user"}}"#;
    let outcome = run_frame(&guardrail, frame).await;
    assert_eq!(
        guardrail.applied(),
        vec![vec![String::from(r#"{"value":"obj"}"#)]]
    );
    assert_eq!(outcome.backend.parsed(), vec![json(frame)]);
}

async fn blocked_tool_output_is_replaced_before_cancellation() {
    let guardrail = ScriptedGuardrail::blocking(vec![GuardrailEventHooks::PreCall], "tool-secret");
    let frame = r#"{"type":"conversation.item.create","item":{"type":"function_call_output","call_id":"call-1","output":"tool-secret","role":"user"}}"#;
    let outcome = ack(Fixture::default()
        .with_guardrail(guardrail.clone())
        .with_client_message(frame)
        .with_client_message(RESPONSE_CREATE))
    .await;
    assert_eq!(guardrail.applied(), vec![vec![String::from("tool-secret")]]);
    let backend = outcome.backend.parsed();
    assert_eq!(backend[0], sanitized_tool_output());
    assert_eq!(backend[1], json(CANCEL));
    assert_eq!(backend.len(), 5);
    assert_eq!(backend[4], json(RESPONSE_CREATE));
    assert_eq!(outcome.state.setup.pending_guardrail_message, None);
}

async fn empty_tool_output_is_not_checked() {
    let guardrail = pre_call_guardrail();
    let frame = r#"{"type":"conversation.item.create","item":{"type":"function_call_output","call_id":"call-1","output":""}}"#;
    let outcome = run_frame(&guardrail, frame).await;
    assert!(guardrail.applied().is_empty());
    assert_eq!(outcome.backend.parsed(), vec![json(frame)]);
}

async fn malformed_and_foreign_content_fall_through() {
    let guardrail = pre_call_guardrail();
    let string_content = r#"{"type":"conversation.item.create","item":{"type":"message","role":"user","content":"plain"}}"#;
    let outcome = ack(Fixture::default()
        .with_guardrail(guardrail.clone())
        .with_client_message("not json")
        .with_client_message("[]")
        .with_client_message(string_content))
    .await;
    assert!(guardrail.applied().is_empty());
    assert_eq!(
        outcome.backend.frames(),
        vec![
            String::from("not json"),
            String::from("[]"),
            String::from(string_content)
        ]
    );
    assert_eq!(outcome.exit, ClientLoopExit::ClientDisconnected);
}

async fn non_iterable_content_ends_the_loop() {
    let (logs, _guard) = capture();
    let frame = r#"{"type":"conversation.item.create","item":{"type":"message","role":"user","content":5}}"#;
    let outcome = ack(Fixture::default()
        .with_guardrail(pre_call_guardrail())
        .with_client_message(frame))
    .await;
    assert_eq!(outcome.exit, ClientLoopExit::ClientDisconnected);
    assert!(outcome.backend.frames().is_empty());
    assert!(logs.starts_with("Error in client ack messages: "));
}

fn sanitized_tool_output() -> Json {
    let output = String::from(r#"{"error":"Tool output blocked by content policy"}"#);
    let mut item = indexmap::IndexMap::new();
    item.insert(
        String::from("type"),
        Json::Text(String::from("function_call_output")),
    );
    item.insert(String::from("call_id"), Json::Text(String::from("call-1")));
    item.insert(String::from("output"), Json::Text(output));
    item.insert(String::from("role"), Json::Text(String::from("user")));
    let mut root = indexmap::IndexMap::new();
    root.insert(
        String::from("type"),
        Json::Text(String::from("conversation.item.create")),
    );
    root.insert(String::from("item"), Json::Object(item));
    Json::Object(root)
}
