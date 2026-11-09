use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::ALLOW_SOURCE;
use super::fake::FakeHost;
use super::fake::LoadReply;
use super::fake::build;
use super::fake::inputs;
use super::fake::map;
use super::fake::object;
use super::fake::params;
use super::fake::text;
use super::fake::value;
use crate::custom_code::CustomCodeInputs;
use crate::generic_guardrail_contracts::GuardrailInputType;

fn host_returning(result: GuardrailValue) -> Arc<FakeHost> {
    FakeHost::new(LoadReply::Module(value(result)))
}

fn allow_result() -> GuardrailValue {
    object(vec![("action", text("allow"))])
}

#[tokio::test]
async fn allow_returns_the_inputs_unchanged() {
    let host = host_returning(allow_result());
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let mut original = inputs(&["hello"]);
    original.extra.insert("model".to_owned(), text("gpt-4o"));
    let mut request_data = BTreeMap::new();
    let returned = guardrail
        .apply_guardrail(
            original.clone(),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(returned, original);
}

#[tokio::test]
async fn a_missing_action_defaults_to_allow() {
    let host = host_returning(object(vec![]));
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let mut request_data = BTreeMap::new();
    let original = inputs(&["hello"]);
    let returned = guardrail
        .apply_guardrail(
            original.clone(),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(returned, original);
}

#[tokio::test]
async fn apply_passes_the_inputs_and_the_safe_request_data_to_the_function() {
    let host = host_returning(allow_result());
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let mut request_data = map(vec![
        ("model", text("gpt-4o")),
        ("api_key", text("sk-secret")),
        ("headers", object(vec![("authorization", text("Bearer x"))])),
        (
            "metadata",
            object(vec![("user_api_key_user_id", text("u1"))]),
        ),
    ]);
    let original = inputs(&["hello"]);
    let _returned = guardrail
        .apply_guardrail(
            original.clone(),
            &mut request_data,
            GuardrailInputType::Response,
        )
        .await
        .unwrap();
    let arguments = host.log.seen.lock().unwrap().clone().unwrap();
    assert_eq!(arguments.inputs, original.to_value());
    assert_eq!(arguments.input_type, GuardrailInputType::Response);
    let GuardrailValue::Object(safe) = arguments.request_data else {
        panic!("expected the safe request data to be an object");
    };
    let keys: Vec<&str> = safe.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["end_user_id", "metadata", "model", "team_id", "user_id"]
    );
    assert_eq!(safe.get("model"), Some(&text("gpt-4o")));
    assert_eq!(safe.get("user_id"), Some(&text("u1")));
    assert!(!safe.contains_key("api_key"));
    assert!(!safe.contains_key("headers"));
}

#[tokio::test]
async fn modify_replaces_the_named_fields_and_keeps_the_others() {
    let result = object(vec![
        ("action", text("modify")),
        (
            "texts",
            GuardrailValue::Array(vec![text("rewritten"), text("second")]),
        ),
        ("images", GuardrailValue::Null),
        (
            "tool_calls",
            GuardrailValue::Array(vec![object(vec![("id", text("call-1"))])]),
        ),
    ]);
    let host = host_returning(result);
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let original = CustomCodeInputs {
        texts: Some(vec!["original".to_owned()]),
        images: Some(vec!["image".to_owned()]),
        tool_calls: None,
        extra: map(vec![("tools", GuardrailValue::Array(Vec::new()))]),
    };
    let mut request_data = BTreeMap::new();
    let returned = guardrail
        .apply_guardrail(
            original.clone(),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(
        returned.texts,
        Some(vec!["rewritten".to_owned(), "second".to_owned()])
    );
    assert_eq!(returned.images, original.images);
    assert_eq!(
        returned.tool_calls,
        Some(vec![object(vec![("id", text("call-1"))])])
    );
    assert_eq!(returned.extra, original.extra);
    assert_eq!(original.extra.len(), 1);
}

#[tokio::test]
async fn modify_replaces_images_with_a_text_list() {
    let result = object(vec![
        ("action", text("modify")),
        ("images", GuardrailValue::Array(vec![text("new-image")])),
    ]);
    let host = host_returning(result);
    let guardrail = build(params(ALLOW_SOURCE), &host);
    let original = CustomCodeInputs {
        texts: None,
        images: Some(vec!["old-image".to_owned()]),
        tool_calls: None,
        extra: BTreeMap::new(),
    };
    let mut request_data = BTreeMap::new();
    let returned = guardrail
        .apply_guardrail(original, &mut request_data, GuardrailInputType::Request)
        .await
        .unwrap();
    assert_eq!(returned.images, Some(vec!["new-image".to_owned()]));
}
