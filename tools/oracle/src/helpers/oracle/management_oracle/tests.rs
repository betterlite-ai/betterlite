#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use super::{Input, run};
use crate::case::parse_helper_case;
use crate::helpers::HelperCall;
use crate::helpers::answers::Answers;

fn decode(value: Value) -> Result<Input, String> {
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn python_call(input: &Input) -> HelperCall {
    HelperCall {
        operation: "management_oracle".to_owned(),
        input: json!(input),
    }
}

fn path_encoding_call(function: &str) -> Value {
    json!({
        "domain": "models",
        "operation": {"path_encoding": {
            "function": function,
            "value": "a/b",
            "field_name": "model_id"
        }}
    })
}

#[test]
fn namespace_native_dispatch() {
    let input = decode(path_encoding_call("segment")).unwrap();
    assert_eq!(run(&input).unwrap(), json!("a%2Fb"));
}

#[test]
fn namespace_python_dispatch() {
    let input = decode(path_encoding_call("segments")).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let answers = Answers::from_env(&root).unwrap();
    let output = answers
        .answer(&root, &python_call(&input), &BTreeMap::new())
        .unwrap();
    assert_eq!(output.value, Some(json!("a/b")));
}

#[test]
fn detects_changed_result() {
    let left = output(json!("left"));
    let right = output(json!("right"));
    assert!(crate::compare::compare_helper(&left, &right, &[]).is_some());
}

#[test]
fn detects_effect_order() {
    let left = output(Value::Null);
    let mut right = left.clone();
    right.effects.reverse();
    assert!(crate::compare::compare_helper(&left, &right, &[]).is_some());
}

#[test]
fn detects_warning_order() {
    let mut left = output(Value::Null);
    left.warnings = vec![
        crate::compare::helper::HelperWarning {
            subject: "first".to_owned(),
            message: "first warning".to_owned(),
        },
        crate::compare::helper::HelperWarning {
            subject: "second".to_owned(),
            message: "second warning".to_owned(),
        },
    ];
    let mut right = left.clone();
    right.warnings.reverse();
    assert!(crate::compare::compare_helper(&left, &right, &[]).is_some());
}

#[test]
fn exception_null_and_code_types() {
    let left: crate::compare::helper::HelperOutput = serde_json::from_str(
        r#"{"ok":false,"error":{"class":"Error","message":"failed","code":null}}"#,
    )
    .unwrap();
    let missing: crate::compare::helper::HelperOutput =
        serde_json::from_str(r#"{"ok":false,"error":{"class":"Error","message":"failed"}}"#)
            .unwrap();
    let numeric: crate::compare::helper::HelperOutput = serde_json::from_str(
        r#"{"ok":false,"error":{"class":"Error","message":"failed","code":401}}"#,
    )
    .unwrap();
    let fractional: crate::compare::helper::HelperOutput = serde_json::from_str(
        r#"{"ok":false,"error":{"class":"Error","message":"failed","code":401.5}}"#,
    )
    .unwrap();
    assert!(crate::compare::compare_helper(&left, &missing, &[]).is_some());
    assert!(crate::compare::compare_helper(&left, &numeric, &[]).is_some());
    assert!(crate::compare::compare_helper(&numeric, &fractional, &[]).is_some());
    serde_json::from_str::<crate::helpers::HelperOutput>(
        r#"{"ok":false,"error":{"class":"Error","message":"failed","code":true}}"#,
    )
    .unwrap_err();
    serde_json::from_str::<crate::helpers::HelperOutput>(
        r#"{"ok":false,"error":{"class":"Error","message":"failed","code":{}}}"#,
    )
    .unwrap_err();
    serde_json::from_str::<crate::helpers::HelperOutput>(
        r#"{"ok":false,"error":{"class":"Error","message":"failed","code":[]}}"#,
    )
    .unwrap_err();
}

#[test]
fn rejects_unknown_domain() {
    let error = decode(json!({"domain": "billing", "operation": {}})).unwrap_err();
    assert!(error.contains("unknown variant"));
}

#[test]
fn rejects_unknown_operation() {
    let error = decode(json!({"domain": "models", "operation": {"list": {}}})).unwrap_err();
    assert!(error.contains("unknown variant"));
}

#[test]
fn rejects_duplicate_fields() {
    let source = "
kind: helper
name: duplicate-management-field
call:
  management_oracle:
    domain: models
    operation:
      path_encoding:
        function: segment
        value: a
        field_name: id
        field_name: other
env: {}
compare:
  ignore: []
";
    let error = parse_helper_case(source).unwrap_err();
    assert!(error.contains("duplicate"));
}

#[test]
fn rejects_unknown_fields() {
    let error = decode(json!({
        "domain": "models",
        "operation": {"path_encoding": {"function": "segment", "value": "a", "field_name": "id", "extra": true}}
    }))
    .unwrap_err();
    assert!(error.contains("unknown field"));
}

#[test]
fn rejects_unknown_ignore() {
    let call = serde_json::from_value(json!({
        "management_oracle": path_encoding_call("segment")
    }))
    .unwrap();
    let actual = crate::helpers::invoke(&call);
    let ignores = vec![crate::case::Ignore {
        pointer: "/value/not_present".to_owned(),
        why: "an unknown pointer must fail even when the results match".to_owned(),
    }];
    assert!(crate::compare::compare_helper(&actual, &actual, &ignores).is_some());
}

#[test]
fn secret_mismatch_still_fails() {
    let mut left = output(Value::Null);
    left.effects.push(effect("credential", "old-secret"));
    let mut right = left.clone();
    right.effects.last_mut().unwrap().fields = credential("new-secret");
    assert!(crate::compare::compare_helper(&left, &right, &[]).is_some());
}

#[test]
fn redacts_secret_artifacts() {
    let mut left = output(Value::Null);
    left.effects.push(effect("credential", "old-secret"));
    let mut right = left.clone();
    right.effects.last_mut().unwrap().fields = credential("new-secret");
    let difference = crate::compare::compare_helper(&left, &right, &[]).unwrap();
    let left_output = serde_json::to_string(&left).unwrap();
    let right_output = serde_json::to_string(&right).unwrap();
    let diagnostic = serde_json::to_string(&difference).unwrap();
    assert!(!left_output.contains("old-secret"));
    assert!(!left_output.contains("new-secret"));
    assert!(!right_output.contains("old-secret"));
    assert!(!right_output.contains("new-secret"));
    assert!(!diagnostic.contains("old-secret"));
    assert!(!diagnostic.contains("new-secret"));
}

#[test]
fn rejects_unknown_effect_fields() {
    let error = serde_json::from_str::<crate::compare::helper::HelperOutput>(
        r#"{"ok":true,"effects":[{"kind":"marker","fields":{"name":"first"},"unexpected":true}]}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("unexpected"));
}

#[test]
fn self_check_is_not_native() {
    assert_eq!(
        crate::helpers::left_side(false),
        crate::helpers::HelperLeft::Native
    );
    assert_eq!(
        crate::helpers::left_side(true),
        crate::helpers::HelperLeft::Python
    );
}

fn output(value: Value) -> crate::compare::helper::HelperOutput {
    crate::compare::helper::HelperOutput {
        ok: true,
        value: Some(value),
        error: None,
        warnings: Vec::new(),
        effects: vec![effect("marker", "first"), effect("marker", "second")],
    }
}

fn effect(kind: &str, value: &str) -> crate::compare::helper::HelperEffect {
    crate::compare::helper::HelperEffect {
        kind: kind.to_owned(),
        fields: if kind == "credential" {
            credential(value)
        } else {
            crate::compare::helper::ManagementEffectFields {
                name: Some(value.to_owned()),
                credential: None,
            }
        },
    }
}

fn credential(value: &str) -> crate::compare::helper::ManagementEffectFields {
    crate::compare::helper::ManagementEffectFields {
        name: None,
        credential: Some(secrecy::SecretString::from(value.to_owned())),
    }
}
