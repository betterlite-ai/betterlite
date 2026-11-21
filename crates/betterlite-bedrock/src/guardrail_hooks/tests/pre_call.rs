use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::fake::base;
use super::support::{array, message, object, text};
use crate::guardrail_hooks::errors::{HookFailure, ModifyFailure};
use crate::guardrail_hooks::input::pre_call_hook;
use crate::guardrail_hooks::runtime::{RoleSelection, ScanSource};

fn request(messages: Vec<GuardrailValue>) -> BTreeMap<String, GuardrailValue> {
    let mut data = BTreeMap::new();
    data.insert("messages".to_owned(), array(messages));
    data
}

#[tokio::test]
async fn scans_the_latest_user_message_and_writes_the_masked_text_back() {
    let mut runtime = base();
    let original = vec![
        message("system", text("be nice")),
        message("user", text("secret")),
    ];
    runtime.messages = Some(original.clone());
    runtime.selection = RoleSelection {
        payload: Some(vec![original[1].clone()]),
        original: Some(original.clone()),
        target_indices: Some(vec![1]),
    };
    runtime.scan_result = Ok(object(vec![("action", text("NONE"))]));
    runtime.updated = Some(vec![message("user", text("masked"))]);
    let mut data = request(original.clone());
    pre_call_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    assert_eq!(
        data["messages"],
        array(vec![original[0].clone(), message("user", text("masked"))])
    );
    assert_eq!(data["applied"], GuardrailValue::Boolean(true));
    let scans = runtime.scans.lock().unwrap();
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0].source, ScanSource::Input);
    assert_eq!(scans[0].event, GuardrailEventHooks::PreCall);
    assert_eq!(scans[0].message_count, 1);
    assert!(!scans[0].has_response);
}

#[tokio::test]
async fn replaces_every_message_when_no_latest_only_filter_ran() {
    let mut runtime = base();
    let original = vec![message("system", text("a")), message("user", text("b"))];
    runtime.messages = Some(original.clone());
    runtime.selection = RoleSelection {
        payload: Some(original.clone()),
        original: None,
        target_indices: None,
    };
    runtime.scan_result = Ok(object(vec![("action", text("NONE"))]));
    runtime.updated = Some(vec![
        message("system", text("a")),
        message("user", text("c")),
    ]);
    let mut data = request(original);
    pre_call_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    assert_eq!(
        data["messages"],
        array(vec![
            message("system", text("a")),
            message("user", text("c"))
        ])
    );
}

#[tokio::test]
async fn leaves_the_request_alone_when_the_guardrail_does_not_run() {
    let mut runtime = base();
    runtime.should_run = false;
    runtime.messages = Some(vec![message("user", text("hi"))]);
    let mut data = request(vec![message("user", text("hi"))]);
    let before = data.clone();
    pre_call_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    assert_eq!(data, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn leaves_the_request_alone_when_the_call_type_has_no_messages() {
    let mut runtime = base();
    let mut data = request(vec![message("user", text("hi"))]);
    let before = data.clone();
    pre_call_hook(&runtime, &mut data, "embedding")
        .await
        .unwrap();
    assert_eq!(data, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn leaves_the_request_alone_when_no_user_message_is_left() {
    let mut runtime = base();
    runtime.messages = Some(vec![message("system", text("hi"))]);
    runtime.selection = RoleSelection {
        payload: Some(Vec::new()),
        original: None,
        target_indices: None,
    };
    let mut data = request(vec![message("system", text("hi"))]);
    let before = data.clone();
    pre_call_hook(&runtime, &mut data, "completion")
        .await
        .unwrap();
    assert_eq!(data, before);
    assert!(runtime.scans.lock().unwrap().is_empty());
}

#[tokio::test]
async fn propagates_a_block_to_the_endpoint_handler() {
    let mut runtime = base();
    runtime.messages = Some(vec![message("user", text("hi"))]);
    runtime.selection = RoleSelection {
        payload: runtime.messages.clone(),
        original: None,
        target_indices: None,
    };
    runtime.scan_result = Err(HookFailure::ModifyResponse(Box::new(ModifyFailure {
        message: "blocked input".to_owned(),
        model: Some("bedrock-model".to_owned()),
        original_response: None,
    })));
    let mut data = request(vec![message("user", text("hi"))]);
    let failure = pre_call_hook(&runtime, &mut data, "completion")
        .await
        .unwrap_err();
    match failure {
        HookFailure::ModifyResponse(failure) => assert_eq!(failure.message, "blocked input"),
        other => panic!("expected a modify response, got {other:?}"),
    }
}
