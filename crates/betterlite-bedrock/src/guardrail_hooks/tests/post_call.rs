use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::base;
use super::support::{array, chat_response, message, object, text, usage};
use crate::guardrail_hooks::errors::{HookFailure, ModifyFailure};
use crate::guardrail_hooks::post_call::post_call_success_hook;
use crate::guardrail_hooks::runtime::ScanSource;

fn request() -> BTreeMap<String, GuardrailValue> {
    let mut data = BTreeMap::new();
    data.insert(
        "messages".to_owned(),
        array(vec![message("user", text("hello"))]),
    );
    data
}

#[tokio::test]
async fn scans_only_the_output_and_masks_the_response() {
    let mut runtime = base();
    runtime.scan_result = Ok(object(vec![("outputs", array(vec![]))]));
    runtime.masked_response = Some(chat_response("gpt-test", text("[MASKED]")));
    let mut data = request();
    let mut response = chat_response("gpt-test", text("secret"));
    post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap();
    assert_eq!(response, chat_response("gpt-test", text("[MASKED]")));
    let scans = runtime.scans.lock().unwrap();
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].source, ScanSource::Output);
    assert!(scans[0].has_response);
    assert_eq!(runtime.masked.lock().unwrap().len(), 1);
    assert_eq!(data["applied"], GuardrailValue::Boolean(true));
}

#[tokio::test]
async fn skips_a_tool_only_response() {
    let mut runtime = base();
    runtime.scan_result = Ok(object(vec![]));
    let mut data = request();
    let mut response = object(vec![
        ("model", text("gpt-test")),
        (
            "choices",
            array(vec![object(vec![
                (
                    "message",
                    object(vec![
                        ("role", text("assistant")),
                        (
                            "tool_calls",
                            array(vec![object(vec![("id", text("call_1"))])]),
                        ),
                    ]),
                ),
                ("finish_reason", text("tool_calls")),
            ])]),
        ),
    ]);
    let before = response.clone();
    post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap();
    assert_eq!(response, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn skips_a_response_whose_text_is_empty() {
    let runtime = base();
    let mut data = request();
    let mut response = chat_response("gpt-test", text(""));
    post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap();
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn scans_a_response_that_is_not_a_model_response() {
    let mut runtime = base();
    runtime.scan_result = Ok(object(vec![]));
    let mut data = request();
    let mut response = object(vec![(
        "choices",
        array(vec![object(vec![("text", text("completion"))])]),
    )]);
    let before = response.clone();
    post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap();
    assert_eq!(response, before);
    assert_eq!(runtime.scans.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn attaches_the_replaced_response_before_rethrowing_a_block() {
    let mut runtime = base();
    runtime.scan_result = Err(HookFailure::ModifyResponse(Box::new(ModifyFailure {
        message: "blocked output".to_owned(),
        model: Some("bedrock-model".to_owned()),
        original_response: None,
    })));
    let mut data = request();
    let mut response = chat_response("gpt-test", text("secret"));
    let failure = post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap_err();
    match failure {
        HookFailure::ModifyResponse(failure) => {
            assert_eq!(failure.message, "blocked output");
            assert_eq!(failure.original_response, Some(response));
        }
        other => panic!("expected a modify response, got {other:?}"),
    }
}

#[tokio::test]
async fn keeps_an_original_response_the_scan_already_attached() {
    let attached = object(vec![("usage", usage(9, 4))]);
    let mut runtime = base();
    runtime.scan_result = Err(HookFailure::ModifyResponse(Box::new(ModifyFailure {
        message: "blocked output".to_owned(),
        model: None,
        original_response: Some(attached.clone()),
    })));
    let mut data = request();
    let mut response = chat_response("gpt-test", text("secret"));
    let failure = post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap_err();
    match failure {
        HookFailure::ModifyResponse(failure) => {
            assert_eq!(failure.original_response, Some(attached));
        }
        other => panic!("expected a modify response, got {other:?}"),
    }
}

#[tokio::test]
async fn leaves_the_response_alone_when_there_are_no_messages() {
    let mut runtime = base();
    runtime.scan_result = Ok(object(vec![]));
    let mut data = BTreeMap::new();
    let mut response = chat_response("gpt-test", text("secret"));
    let before = response.clone();
    post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap();
    assert_eq!(response, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn leaves_the_response_alone_when_the_guardrail_does_not_run() {
    let mut runtime = base();
    runtime.should_run = false;
    let mut data = request();
    let mut response = chat_response("gpt-test", text("secret"));
    let before = response.clone();
    post_call_success_hook(&runtime, &mut data, &mut response)
        .await
        .unwrap();
    assert_eq!(response, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}
