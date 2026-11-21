use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::fake::base;
use super::support::{array, message, object, text};
use crate::guardrail_hooks::errors::{HookFailure, ModifyFailure};
use crate::guardrail_hooks::input::moderation_hook;
use crate::guardrail_hooks::runtime::RoleSelection;

fn request(messages: Vec<GuardrailValue>) -> BTreeMap<String, GuardrailValue> {
    let mut data = BTreeMap::new();
    data.insert("messages".to_owned(), array(messages));
    data
}

fn scripted_runtime(kind: &str) -> super::fake::FakeRuntime {
    let mut runtime = base();
    let messages = vec![message("user", text(kind))];
    runtime.messages = Some(messages.clone());
    runtime.selection = RoleSelection {
        payload: Some(messages.clone()),
        original: None,
        target_indices: None,
    };
    runtime.scan_result = Ok(object(vec![("action", text("NONE"))]));
    runtime
}

#[tokio::test]
async fn uses_the_during_call_event_for_chat_calls() {
    let runtime = scripted_runtime("hi");
    let mut data = request(vec![message("user", text("hi"))]);
    moderation_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    let scans = runtime.scans.lock().unwrap();
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].event, GuardrailEventHooks::DuringCall);
    assert_eq!(data["applied"], GuardrailValue::Boolean(true));
}

#[tokio::test]
async fn uses_the_during_mcp_call_event_for_the_mcp_call_type() {
    let runtime = scripted_runtime("hi");
    let mut data = request(vec![message("user", text("hi"))]);
    moderation_hook(&runtime, &mut data, "call_mcp_tool")
        .await
        .unwrap();
    let scans = runtime.scans.lock().unwrap();
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].event, GuardrailEventHooks::DuringMcpCall);
}

#[tokio::test]
async fn writes_the_masked_message_back_into_the_request() {
    let mut runtime = scripted_runtime("secret");
    runtime.updated = Some(vec![message("user", text("masked"))]);
    let mut data = request(vec![message("user", text("secret"))]);
    moderation_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    assert_eq!(
        data["messages"],
        array(vec![message("user", text("masked"))])
    );
}

#[tokio::test]
async fn lets_a_modify_response_propagate_while_the_llm_call_runs() {
    let mut runtime = scripted_runtime("hi");
    runtime.scan_result = Err(HookFailure::ModifyResponse(Box::new(ModifyFailure {
        message: "blocked during call".to_owned(),
        model: None,
        original_response: None,
    })));
    let mut data = request(vec![message("user", text("hi"))]);
    let failure = moderation_hook(&runtime, &mut data, "completion")
        .await
        .unwrap_err();
    match failure {
        HookFailure::ModifyResponse(failure) => assert_eq!(failure.message, "blocked during call"),
        other => panic!("expected a modify response, got {other:?}"),
    }
}

#[tokio::test]
async fn leaves_the_request_alone_when_the_guardrail_does_not_run() {
    let mut runtime = scripted_runtime("hi");
    runtime.should_run = false;
    let mut data = request(vec![message("user", text("hi"))]);
    let before = data.clone();
    moderation_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    assert_eq!(data, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn leaves_the_request_alone_when_the_call_type_has_no_messages() {
    let runtime = base();
    let mut data = request(vec![message("user", text("hi"))]);
    let before = data.clone();
    moderation_hook(&runtime, &mut data, "embedding")
        .await
        .unwrap();
    assert_eq!(data, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}
