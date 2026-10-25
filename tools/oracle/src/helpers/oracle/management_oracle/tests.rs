#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{Input, run};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchMode {
    NativeThenPython,
    PythonThenPython,
}

#[derive(Debug, Clone, PartialEq)]
struct ContractOutput {
    result: Value,
    warnings: Vec<Value>,
    effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq)]
struct Effect {
    kind: String,
    fields: BTreeMap<String, Value>,
    secret_fields: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct ContractDifference {
    pointer: String,
    diagnostic: String,
}

impl Effect {
    fn credential(value: &str) -> Self {
        Self::new("credential_written", "credential", value, true)
    }

    fn marker(name: &str) -> Self {
        Self::new("marker", "name", name, false)
    }

    fn new(kind: &str, field: &str, value: &str, secret: bool) -> Self {
        let mut fields = BTreeMap::new();
        fields.insert(field.to_owned(), Value::String(value.to_owned()));
        let secret_fields = if secret {
            BTreeSet::from([field.to_owned()])
        } else {
            BTreeSet::new()
        };
        Self {
            kind: kind.to_owned(),
            fields,
            secret_fields,
        }
    }
}

fn decode(value: Value) -> Result<Input, String> {
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn python_request(input: &Input) -> Value {
    json!({"operation": "management_oracle", "input": input})
}

const fn dispatch_mode(self_check: bool) -> DispatchMode {
    if self_check {
        DispatchMode::PythonThenPython
    } else {
        DispatchMode::NativeThenPython
    }
}

fn compare_contracts(
    left: &ContractOutput,
    right: &ContractOutput,
    ignores: &[String],
) -> Result<(), ContractDifference> {
    let mut left_value = output_value(left);
    let mut right_value = output_value(right);
    for pointer in ignores {
        if left_value.pointer(pointer).is_none() && right_value.pointer(pointer).is_none() {
            return Err(ContractDifference {
                pointer: pointer.clone(),
                diagnostic: "compare.ignore points to no field".to_owned(),
            });
        }
        remove_pointer(&mut left_value, pointer);
        remove_pointer(&mut right_value, pointer);
    }
    if left_value == right_value {
        return Ok(());
    }
    let mut redacted_left = output_value(left);
    let mut redacted_right = output_value(right);
    redact_effect_secrets(left, &mut redacted_left);
    redact_effect_secrets(right, &mut redacted_right);
    Err(ContractDifference {
        pointer: "/".to_owned(),
        diagnostic: format!(
            "management results differ: left={redacted_left} right={redacted_right}"
        ),
    })
}

fn output_value(output: &ContractOutput) -> Value {
    let effects = output
        .effects
        .iter()
        .map(|effect| json!({"kind": effect.kind, "fields": effect.fields}))
        .collect::<Vec<_>>();
    json!({"result": output.result, "warnings": output.warnings, "effects": effects})
}

fn redact_effect_secrets(output: &ContractOutput, value: &mut Value) {
    for (index, effect) in output.effects.iter().enumerate() {
        for field in &effect.secret_fields {
            let pointer = format!("/effects/{index}/fields/{}", escape_pointer(field));
            if let Some(secret) = value.pointer_mut(&pointer) {
                *secret = Value::String("[REDACTED]".to_owned());
            }
        }
    }
}

fn escape_pointer(value: &str) -> String {
    let escaped = value.replace('~', "~0");
    escaped.replace('/', "~1")
}

fn remove_pointer(value: &mut Value, pointer: &str) {
    if pointer.is_empty() {
        *value = Value::Null;
    } else if let Some((parent, raw_key)) = pointer.rsplit_once('/') {
        let decoded_key = raw_key.replace("~1", "/").replace("~0", "~");
        if let Some(Value::Object(object)) = value.pointer_mut(parent) {
            object.remove(&decoded_key);
        }
    } else {
        debug_assert!(!pointer.is_empty() && !pointer.contains('/'));
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
    assert_eq!(
        python_request(&input),
        json!({"operation": "management_oracle", "input": path_encoding_call("segments")})
    );
}

#[test]
fn detects_changed_result() {
    let left = output(json!({"ok": true, "value": "left"}));
    let right = output(json!({"ok": true, "value": "right"}));
    assert!(compare_contracts(&left, &right, &[]).is_err());
}

#[test]
fn detects_effect_order() {
    let left = output(json!({"ok": true, "value": null}));
    let mut right = left.clone();
    right.effects.reverse();
    assert!(compare_contracts(&left, &right, &[]).is_err());
}

#[test]
fn detects_warning_order() {
    let left = ContractOutput {
        result: json!({"ok": true}),
        warnings: vec![json!("first"), json!("second")],
        effects: Vec::new(),
    };
    let mut right = left.clone();
    right.warnings.reverse();
    assert!(compare_contracts(&left, &right, &[]).is_err());
}

#[test]
fn exception_null_and_code_types() {
    let left = output(json!({"ok": false, "error": {"code": null}}));
    let missing = output(json!({"ok": false, "error": {}}));
    let numeric = output(json!({"ok": false, "error": {"code": 401_i32}}));
    assert!(compare_contracts(&left, &missing, &[]).is_err());
    assert!(compare_contracts(&left, &numeric, &[]).is_err());
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
    let value = serde_json::from_str::<Input>(
        r#"{"domain":"models","operation":{"path_encoding":{"function":"segment","value":"a","field_name":"id","field_name":"other"}}}"#,
    );
    let _error = value.unwrap_err();
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
fn helper_case_decoder_rejects_management_unknown_fields() {
    let source = "kind: helper\nname: invalid\ncall: {management_oracle: {domain: models, operation: {path_encoding: {function: segment, value: a, field_name: id, extra: true}}}}\nenv: {}\ncompare: {ignore: []}\n";
    let error = crate::case::parse_helper_case(source).unwrap_err();
    assert!(error.contains("unknown field"));
}

#[test]
fn rejects_unknown_ignore() {
    let left = output(json!({"ok": true}));
    let right = left.clone();
    let ignore = vec!["/result/not_present".to_owned()];
    assert!(compare_contracts(&left, &right, &ignore).is_err());
}

#[test]
fn secret_mismatch_still_fails() {
    let mut left = output(json!({"ok": true}));
    left.effects.push(Effect::credential("old-secret"));
    let mut right = left.clone();
    right
        .effects
        .last_mut()
        .unwrap()
        .fields
        .insert("credential".to_owned(), json!("new-secret"));
    assert!(compare_contracts(&left, &right, &[]).is_err());
}

#[test]
fn redacts_secret_artifacts() {
    let mut left = output(json!({"ok": true}));
    left.effects.push(Effect::credential("old-secret"));
    let mut right = left.clone();
    right
        .effects
        .last_mut()
        .unwrap()
        .fields
        .insert("credential".to_owned(), json!("new-secret"));
    let difference = compare_contracts(&left, &right, &[]).unwrap_err();
    assert!(!difference.diagnostic.contains("old-secret"));
    assert!(!difference.diagnostic.contains("new-secret"));
}

#[test]
fn self_check_is_not_native() {
    assert_eq!(dispatch_mode(false), DispatchMode::NativeThenPython);
    assert_eq!(dispatch_mode(true), DispatchMode::PythonThenPython);
}

fn output(result: Value) -> ContractOutput {
    ContractOutput {
        result,
        warnings: Vec::new(),
        effects: vec![Effect::marker("first"), Effect::marker("second")],
    }
}
