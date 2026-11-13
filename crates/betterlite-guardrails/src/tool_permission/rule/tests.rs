#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::{ToolDecision, ToolPermissionRule};
use crate::guardrail_initializers::tool_permission::{
    ToolDefaultAction, ToolDisallowedAction, ToolPermissionArgs,
};
use crate::tool_permission::ToolPermissionGuardrail;
use crate::tool_permission::error::ToolPermissionError;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn fields(entries: Vec<(&str, GuardrailValue)>) -> BTreeMap<String, GuardrailValue> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn rule_fields(
    id: &str,
    decision: &str,
    tool_name: Option<&str>,
    tool_type: Option<&str>,
) -> BTreeMap<String, GuardrailValue> {
    let mut fields = fields(vec![("id", text(id)), ("decision", text(decision))]);
    if let Some(tool_name) = tool_name {
        fields.insert("tool_name".to_owned(), text(tool_name));
    }
    if let Some(tool_type) = tool_type {
        fields.insert("tool_type".to_owned(), text(tool_type));
    }
    fields
}

fn args(rules: Vec<BTreeMap<String, GuardrailValue>>) -> ToolPermissionArgs {
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
fn rule_requires_at_least_a_target() {
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("no-target")),
        ("decision", text("allow")),
    ]))
    .unwrap_err();
    assert!(matches!(error, ToolPermissionError::MissingTarget));
}

#[rstest]
#[case::spaces("   ")]
#[case::tabs("\t\t")]
#[case::empty("")]
fn rule_treats_a_blank_target_as_absent(#[case] target: &str) {
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("blank")),
        ("decision", text("allow")),
        ("tool_name", text(target)),
    ]))
    .unwrap_err();
    assert!(matches!(error, ToolPermissionError::MissingTarget));
}

#[test]
fn rule_trims_a_target_pattern() {
    let rule = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("read")),
        ("decision", text("allow")),
        ("tool_name", text("  ^read_.*$  ")),
    ]))
    .unwrap();
    assert_eq!(rule.tool_name.as_deref(), Some("^read_.*$"));
    assert_eq!(rule.tool_type, None);
    assert_eq!(rule.allowed_param_patterns, None);
}

#[rstest]
#[case::upper_allow("ALLOW", ToolDecision::Allow)]
#[case::mixed_deny("Deny", ToolDecision::Deny)]
#[case::lower_allow("allow", ToolDecision::Allow)]
#[case::lower_deny("deny", ToolDecision::Deny)]
fn rule_normalizes_the_decision_to_lowercase(
    #[case] decision: &str,
    #[case] expected: ToolDecision,
) {
    let rule =
        ToolPermissionRule::from_fields(&rule_fields("case", decision, Some("^read$"), None))
            .unwrap();
    assert_eq!(rule.decision, expected);
}

#[test]
fn rule_rejects_an_unknown_decision() {
    let error = ToolPermissionRule::from_fields(&rule_fields("bad", "maybe", Some("^read$"), None))
        .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::InvalidDecision { value } if value == "maybe"
    ));
}

#[test]
fn rule_requires_an_id() {
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("decision", text("allow")),
        ("tool_name", text("^read$")),
    ]))
    .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::MissingField { field } if *field == "id"
    ));
}

#[test]
fn rule_requires_a_decision() {
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("no-decision")),
        ("tool_name", text("^read$")),
    ]))
    .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::MissingField { field } if *field == "decision"
    ));
}

#[test]
fn rule_rejects_a_non_string_target() {
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("typed")),
        ("decision", text("allow")),
        ("tool_name", GuardrailValue::UnsignedInteger(7)),
    ]))
    .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::WrongType { field, .. } if field.as_str() == "tool_name"
    ));
}

#[test]
fn rule_accepts_a_null_target_when_the_other_is_present() {
    let rule = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("typed")),
        ("decision", text("allow")),
        ("tool_name", GuardrailValue::Null),
        ("tool_type", text("function")),
    ]))
    .unwrap();
    assert_eq!(rule.tool_name, None);
    assert_eq!(rule.tool_type.as_deref(), Some("function"));
}

#[test]
fn rule_rejects_an_unknown_field() {
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("extra")),
        ("decision", text("allow")),
        ("tool_name", text("^read$")),
        ("action", text("allow")),
    ]))
    .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::UnknownField { field } if field.as_str() == "action"
    ));
}

#[test]
fn rule_parses_the_parameter_patterns() {
    let patterns =
        GuardrailValue::Object(BTreeMap::from([("path".to_owned(), text("^/safe/.*$"))]));
    let rule = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("params")),
        ("decision", text("allow")),
        ("tool_name", text("^read$")),
        ("allowed_param_patterns", patterns),
    ]))
    .unwrap();
    assert_eq!(
        rule.allowed_param_patterns,
        Some(BTreeMap::from([(
            "path".to_owned(),
            "^/safe/.*$".to_owned()
        )]))
    );
}

#[test]
fn rule_rejects_a_non_string_parameter_pattern() {
    let patterns = GuardrailValue::Object(BTreeMap::from([(
        "path".to_owned(),
        GuardrailValue::UnsignedInteger(1),
    )]));
    let error = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("params")),
        ("decision", text("allow")),
        ("tool_name", text("^read$")),
        ("allowed_param_patterns", patterns),
    ]))
    .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::WrongType { field, .. } if field.as_str() == "allowed_param_patterns.path"
    ));
}

#[test]
fn rule_treats_a_null_parameter_map_as_absent() {
    let rule = ToolPermissionRule::from_fields(&fields(vec![
        ("id", text("params")),
        ("decision", text("allow")),
        ("tool_name", text("^read$")),
        ("allowed_param_patterns", GuardrailValue::Null),
    ]))
    .unwrap();
    assert_eq!(rule.allowed_param_patterns, None);
}

#[test]
fn rule_deserializes_the_config_shape() {
    let rule: ToolPermissionRule =
        serde_json::from_str(r#"{"id": "read", "tool_name": "  ^read$  ", "decision": "ALLOW"}"#)
            .unwrap();
    assert_eq!(rule.id, "read");
    assert_eq!(rule.tool_name.as_deref(), Some("^read$"));
    assert_eq!(rule.decision, ToolDecision::Allow);
}

#[test]
fn rule_deserialization_rejects_an_unknown_field() {
    let error = serde_json::from_str::<ToolPermissionRule>(
        r#"{"id": "read", "tool_name": "^read$", "decision": "allow", "action": "deny"}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("action"));
}

#[test]
fn invalid_tool_name_regex_names_the_rule_and_pattern() {
    let error =
        ToolPermissionGuardrail::new(args(vec![rule_fields("broken", "allow", Some("("), None)]))
            .unwrap_err();
    match error {
        ToolPermissionError::InvalidRegex {
            rule_id,
            field,
            pattern,
            ..
        } => {
            assert_eq!(rule_id, "broken");
            assert_eq!(field, "tool_name");
            assert_eq!(pattern, "(");
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn invalid_tool_type_regex_names_the_rule_and_pattern() {
    let error = ToolPermissionGuardrail::new(args(vec![rule_fields(
        "broken",
        "deny",
        None,
        Some("[unclosed"),
    )]))
    .unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::InvalidRegex { rule_id, field, pattern, .. }
            if rule_id == "broken" && field == "tool_type" && pattern == "[unclosed"
    ));
}

#[test]
fn invalid_parameter_regex_names_the_rule_and_pattern() {
    let mut rule = rule_fields("broken", "allow", Some("^read$"), None);
    rule.insert(
        "allowed_param_patterns".to_owned(),
        GuardrailValue::Object(BTreeMap::from([("path".to_owned(), text("("))])),
    );
    let error = ToolPermissionGuardrail::new(args(vec![rule])).unwrap_err();
    assert!(matches!(
        &error,
        ToolPermissionError::InvalidRegex { rule_id, field, pattern, .. }
            if rule_id == "broken" && field == "allowed_param_patterns" && pattern == "("
    ));
}
