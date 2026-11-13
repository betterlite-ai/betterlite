#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;

use super::{ToolDefaultAction, ToolDisallowedAction, initialize_tool_permission};
use crate::guardrail_initializers::error::InitializerError;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn build(extras: &str) -> Result<super::ToolPermissionArgs, InitializerError> {
    let body = format!(r#"{{"guardrail": "tool_permission", "mode": "pre_call"{extras}}}"#);
    initialize_tool_permission(&params(&body), Some("tools-check"))
}

#[test]
fn tool_permission_args_copy_the_listed_fields() {
    let built = build(
        r#",
        "rules": [
            {"id": "allow-read", "action": "allow", "tool_name": "^read$"},
            {"id": "deny-write", "action": "deny", "tool_name": "^write$"}
        ],
        "default_action": "allow",
        "on_disallowed_action": "rewrite",
        "default_on": true,
        "violation_message_template": "blocked {tool_name}"
        "#,
    )
    .unwrap();
    let rules = built.rules.unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules[0].get("id"),
        Some(&GuardrailValue::Text("allow-read".to_owned()))
    );
    assert_eq!(
        rules[1].get("tool_name"),
        Some(&GuardrailValue::Text("^write$".to_owned()))
    );
    assert_eq!(built.default_action, ToolDefaultAction::Allow);
    assert_eq!(built.on_disallowed_action, ToolDisallowedAction::Rewrite);
    assert!(built.default_on);
    assert_eq!(
        built.violation_message_template.as_deref(),
        Some("blocked {tool_name}")
    );
    assert_eq!(built.name, "tools-check");
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn tool_permission_defaults_to_deny_and_block_with_no_rules() {
    let built = build("").unwrap();
    assert_eq!(built.rules, None);
    assert_eq!(built.default_action, ToolDefaultAction::Deny);
    assert_eq!(built.on_disallowed_action, ToolDisallowedAction::Block);
    assert!(!built.default_on);
    assert_eq!(built.violation_message_template, None);
}

#[test]
fn tool_permission_treats_empty_rules_as_none() {
    assert_eq!(build(r#", "rules": []"#).unwrap().rules, None);
    assert_eq!(build(r#", "rules": null"#).unwrap().rules, None);
}

#[test]
fn tool_permission_keeps_rule_order() {
    let built =
        build(r#", "rules": [{"id": "first"}, {"id": "second"}, {"id": "third"}]"#).unwrap();
    let rules = built.rules.unwrap();
    let ids: Vec<&GuardrailValue> = rules.iter().filter_map(|rule| rule.get("id")).collect();
    assert_eq!(
        ids,
        vec![
            &GuardrailValue::Text("first".to_owned()),
            &GuardrailValue::Text("second".to_owned()),
            &GuardrailValue::Text("third".to_owned()),
        ]
    );
}

#[rstest]
#[case::unknown_default(r#", "default_action": "block""#, "default_action")]
#[case::unknown_disallowed(r#", "on_disallowed_action": "drop""#, "on_disallowed_action")]
#[case::rules_not_a_list(r#", "rules": {"id": "one"}"#, "rules")]
#[case::rule_not_an_object(r#", "rules": ["allow"]"#, "rules")]
fn bad_tool_permission_extras_name_the_field(#[case] extras: &str, #[case] field: &str) {
    let error = build(extras).unwrap_err();
    assert!(
        matches!(&error, InitializerError::InvalidParam(name) if name.as_str() == field),
        "unexpected error {error}"
    );
}
