#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use tracing::subscriber::with_default;

use super::ToolPermissionGuardrail;
use super::error::ToolPermissionError;
use super::types::{
    PermissionError, ToolResult, is_tool_use_block, mapping_value, object_list, object_mapping,
};
use crate::guardrail_initializers::tool_permission::{
    ToolDefaultAction, ToolDisallowedAction, ToolPermissionArgs,
};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn rule_fields(
    id: &str,
    decision: &str,
    tool_name: Option<&str>,
    tool_type: Option<&str>,
) -> BTreeMap<String, GuardrailValue> {
    let mut fields = BTreeMap::from([
        ("id".to_owned(), text(id)),
        ("decision".to_owned(), text(decision)),
    ]);
    if let Some(tool_name) = tool_name {
        fields.insert("tool_name".to_owned(), text(tool_name));
    }
    if let Some(tool_type) = tool_type {
        fields.insert("tool_type".to_owned(), text(tool_type));
    }
    fields
}

fn guardrail_args(rules: Vec<BTreeMap<String, GuardrailValue>>) -> ToolPermissionArgs {
    ToolPermissionArgs {
        name: "tools-check".to_owned(),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        rules: Some(rules),
        default_action: ToolDefaultAction::Deny,
        on_disallowed_action: ToolDisallowedAction::Block,
        default_on: false,
        violation_message_template: None,
    }
}

#[test]
fn init_logs_the_rule_count_and_default_action_once() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guardrail = with_default(subscriber, || {
        ToolPermissionGuardrail::new(guardrail_args(vec![
            rule_fields("first", "allow", Some("^read$"), None),
            rule_fields("second", "deny", Some("^write$"), None),
        ]))
    })
    .expect("two valid rules");
    assert_eq!(guardrail.rules.len(), 2);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert_eq!(
        output
            .matches("Tool Permission Guardrail initialized")
            .count(),
        1,
        "unexpected log output {output}"
    );
    assert!(output.contains("rules=2"), "missing rule count in {output}");
    assert!(
        output.contains("default_action=deny"),
        "missing default action in {output}"
    );
}

#[test]
fn init_without_rules_has_no_compiled_state() {
    let mut args = guardrail_args(Vec::new());
    args.rules = None;
    let guardrail = ToolPermissionGuardrail::new(args).expect("no rules is valid");
    assert!(guardrail.rules.is_empty());
    assert!(guardrail.compiled.targets.is_empty());
    assert!(guardrail.compiled.patterns.is_empty());
}

#[test]
fn init_keeps_the_configured_actions() {
    let mut args = guardrail_args(Vec::new());
    args.default_action = ToolDefaultAction::Allow;
    args.on_disallowed_action = ToolDisallowedAction::Rewrite;
    let guardrail = ToolPermissionGuardrail::new(args).expect("no rules is valid");
    assert_eq!(guardrail.args.default_action, ToolDefaultAction::Allow);
    assert_eq!(
        guardrail.args.on_disallowed_action,
        ToolDisallowedAction::Rewrite
    );
}

#[test]
fn load_rules_keeps_the_prior_state_when_a_pattern_does_not_compile() {
    let mut guardrail = ToolPermissionGuardrail::new(guardrail_args(vec![rule_fields(
        "keep",
        "allow",
        Some("^keep$"),
        None,
    )]))
    .expect("one valid rule");
    let error = guardrail
        .load_rules(Some(&[
            rule_fields("new-ok", "allow", Some("^ok$"), None),
            rule_fields("broken", "deny", Some("("), None),
        ]))
        .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::InvalidRegex { rule_id, .. } if rule_id == "broken"
    ));
    assert_eq!(guardrail.rules.len(), 1);
    assert_eq!(guardrail.rules[0].id, "keep");
    assert!(guardrail.compiled.targets.contains_key("keep"));
    assert!(!guardrail.compiled.targets.contains_key("new-ok"));
    assert!(!guardrail.compiled.targets.contains_key("broken"));
}

#[test]
fn load_rules_swaps_the_whole_ruleset() {
    let mut guardrail = ToolPermissionGuardrail::new(guardrail_args(vec![rule_fields(
        "old",
        "allow",
        Some("^old$"),
        None,
    )]))
    .expect("one valid rule");
    guardrail
        .load_rules(Some(&[rule_fields(
            "new",
            "deny",
            None,
            Some("^function$"),
        )]))
        .expect("one valid rule");
    assert_eq!(guardrail.rules.len(), 1);
    assert_eq!(guardrail.rules[0].id, "new");
    assert!(!guardrail.compiled.targets.contains_key("old"));
    assert!(guardrail.compiled.targets.contains_key("new"));
}

#[test]
fn load_rules_without_rules_clears_the_ruleset() {
    let mut guardrail = ToolPermissionGuardrail::new(guardrail_args(vec![rule_fields(
        "old",
        "allow",
        Some("^old$"),
        None,
    )]))
    .expect("one valid rule");
    guardrail.load_rules(None).expect("no rules is valid");
    assert!(guardrail.rules.is_empty());
    assert!(guardrail.compiled.targets.is_empty());
    assert!(guardrail.compiled.patterns.is_empty());
}

#[test]
fn object_mapping_returns_the_fields_of_an_object() {
    let value = GuardrailValue::Object(BTreeMap::from([("type".to_owned(), text("tool_use"))]));
    let mapping = object_mapping(&value).expect("an object");
    assert_eq!(mapping.get("type"), Some(&text("tool_use")));
}

#[test]
fn object_mapping_rejects_a_non_object() {
    assert!(object_mapping(&GuardrailValue::Null).is_none());
    assert!(object_mapping(&text("tool_use")).is_none());
    assert!(object_mapping(&GuardrailValue::Array(Vec::new())).is_none());
}

#[test]
fn object_list_returns_the_items_of_a_list() {
    let value = GuardrailValue::Array(vec![text("first"), text("second")]);
    let items = object_list(&value).expect("a list");
    assert_eq!(items, [text("first"), text("second")].as_slice());
}

#[test]
fn object_list_rejects_a_non_list() {
    assert!(object_list(&GuardrailValue::Null).is_none());
    assert!(object_list(&text("first")).is_none());
    assert!(object_list(&GuardrailValue::Object(BTreeMap::new())).is_none());
}

#[test]
fn mapping_value_reads_a_present_key() {
    let value = GuardrailValue::Object(BTreeMap::from([("name".to_owned(), text("read"))]));
    assert_eq!(mapping_value(&value, "name"), Some(&text("read")));
}

#[test]
fn mapping_value_returns_none_for_a_missing_key() {
    let value = GuardrailValue::Object(BTreeMap::from([("name".to_owned(), text("read"))]));
    assert_eq!(mapping_value(&value, "type"), None);
}

#[test]
fn mapping_value_returns_none_for_a_non_object() {
    assert_eq!(mapping_value(&text("read"), "name"), None);
    assert_eq!(mapping_value(&GuardrailValue::Null, "name"), None);
}

#[test]
fn is_tool_use_block_recognizes_the_anthropic_block() {
    let value = GuardrailValue::Object(BTreeMap::from([
        ("type".to_owned(), text("tool_use")),
        ("name".to_owned(), text("read_file")),
    ]));
    assert!(is_tool_use_block(&value));
}

#[test]
fn is_tool_use_block_rejects_other_blocks() {
    let text_block = GuardrailValue::Object(BTreeMap::from([("type".to_owned(), text("text"))]));
    assert!(!is_tool_use_block(&text_block));
    let missing_type = GuardrailValue::Object(BTreeMap::from([("name".to_owned(), text("read"))]));
    assert!(!is_tool_use_block(&missing_type));
    assert!(!is_tool_use_block(&text("tool_use")));
}

#[test]
fn tool_result_defaults_to_a_tool_result_error() {
    let result: ToolResult =
        serde_json::from_str(r#"{"tool_use_id": "call_1", "content": "denied"}"#)
            .expect("tool result");
    assert_eq!(result.kind, "tool_result");
    assert!(result.is_error);
}

#[test]
fn tool_result_reads_an_explicit_success() {
    let result: ToolResult = serde_json::from_str(
        r#"{"type": "tool_result", "tool_use_id": "call_1", "content": "ok", "is_error": false}"#,
    )
    .expect("tool result");
    assert_eq!(result.kind, "tool_result");
    assert!(!result.is_error);
}

#[test]
fn permission_error_keeps_the_rule_id_absent() {
    let error: PermissionError =
        serde_json::from_str(r#"{"tool_name": "shell", "rule_id": null, "message": "denied"}"#)
            .expect("permission error");
    assert_eq!(error.rule_id, None);
    assert_eq!(error.tool_name, "shell");
}
