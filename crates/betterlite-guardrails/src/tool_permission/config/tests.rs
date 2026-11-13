#![cfg(test)]

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;

use super::{
    ToolPermissionConfigModel, default_action_text, disallowed_action_text, parse_default_action,
    parse_disallowed_action,
};
use crate::guardrail_initializers::tool_permission::{ToolDefaultAction, ToolDisallowedAction};
use crate::tool_permission::SUPPORTED_EVENT_HOOKS;
use crate::tool_permission::rule::ToolDecision;

fn model(text: &str) -> ToolPermissionConfigModel {
    serde_json::from_str(text).expect("config model")
}

#[test]
fn config_model_defaults_to_deny_and_block() {
    let parsed = model("{}");
    assert_eq!(parsed.default_action, "deny");
    assert_eq!(parsed.on_disallowed_action, "block");
    let settings = parsed.resolve().expect("resolved settings");
    assert_eq!(settings.default_action, ToolDefaultAction::Deny);
    assert_eq!(settings.on_disallowed_action, ToolDisallowedAction::Block);
    assert_eq!(parsed.rules, None);
    assert_eq!(parsed.optional_params, None);
}

#[rstest]
#[case::upper(
    r#"{"default_action": "ALLOW", "on_disallowed_action": "REWRITE"}"#,
    ToolDefaultAction::Allow,
    ToolDisallowedAction::Rewrite
)]
#[case::mixed(
    r#"{"default_action": "Deny", "on_disallowed_action": "Block"}"#,
    ToolDefaultAction::Deny,
    ToolDisallowedAction::Block
)]
fn config_model_normalizes_the_actions_to_lowercase(
    #[case] text: &str,
    #[case] expected_default: ToolDefaultAction,
    #[case] expected_disallowed: ToolDisallowedAction,
) {
    let settings = model(text).resolve().expect("resolved settings");
    assert_eq!(settings.default_action, expected_default);
    assert_eq!(settings.on_disallowed_action, expected_disallowed);
}

#[rstest]
#[case::default_action(r#"{"default_action": "drop"}"#, "default_action")]
#[case::on_disallowed_action(r#"{"on_disallowed_action": "ignore"}"#, "on_disallowed_action")]
fn config_model_rejects_an_unknown_action(#[case] text: &str, #[case] field: &str) {
    let error = model(text).resolve().unwrap_err();
    assert!(
        error.to_string().contains(field),
        "unexpected error {error}"
    );
}

#[test]
fn config_model_rejects_an_unknown_field() {
    let error =
        serde_json::from_str::<ToolPermissionConfigModel>(r#"{"guardrail": "tool_permission"}"#)
            .unwrap_err();
    assert!(error.to_string().contains("guardrail"));
}

#[test]
fn config_model_parses_rules_through_the_rule_model() {
    let parsed =
        model(r#"{"rules": [{"id": "read", "tool_name": "  ^read$ ", "decision": "ALLOW"}]}"#);
    let rules = parsed.rules.expect("rules");
    assert_eq!(rules[0].tool_name.as_deref(), Some("^read$"));
    assert_eq!(rules[0].decision, ToolDecision::Allow);
}

#[test]
fn config_model_rejects_a_rule_without_a_target() {
    let error = serde_json::from_str::<ToolPermissionConfigModel>(
        r#"{"rules": [{"id": "read", "decision": "allow"}]}"#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("tool_name or tool_type"),
        "unexpected error {error}"
    );
}

#[test]
fn action_text_matches_litellm() {
    assert_eq!(default_action_text(ToolDefaultAction::Allow), "allow");
    assert_eq!(default_action_text(ToolDefaultAction::Deny), "deny");
    assert_eq!(disallowed_action_text(ToolDisallowedAction::Block), "block");
    assert_eq!(
        disallowed_action_text(ToolDisallowedAction::Rewrite),
        "rewrite"
    );
}

#[test]
fn config_model_serializes_the_actions_as_text() {
    let text = serde_json::to_string(&model("{}")).expect("config model");
    assert!(text.contains(r#""default_action":"deny""#), "{text}");
    assert!(text.contains(r#""on_disallowed_action":"block""#), "{text}");
}

#[test]
fn config_model_ui_friendly_name_matches_litellm() {
    assert_eq!(
        ToolPermissionConfigModel::ui_friendly_name(),
        "LiteLLM Tool Permission Guardrail"
    );
}

#[test]
fn supported_event_hooks_are_pre_call_and_post_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall]
    );
}

#[rstest]
#[case::upper_allow("ALLOW", ToolDefaultAction::Allow)]
#[case::lower_deny("deny", ToolDefaultAction::Deny)]
fn parse_default_action_accepts_any_case(#[case] text: &str, #[case] expected: ToolDefaultAction) {
    assert_eq!(parse_default_action(text).unwrap(), expected);
}

#[test]
fn parse_default_action_rejects_an_unknown_value() {
    assert!(parse_default_action("drop").is_err());
}

#[rstest]
#[case::upper_block("BLOCK", ToolDisallowedAction::Block)]
#[case::lower_rewrite("rewrite", ToolDisallowedAction::Rewrite)]
fn parse_disallowed_action_accepts_any_case(
    #[case] text: &str,
    #[case] expected: ToolDisallowedAction,
) {
    assert_eq!(parse_disallowed_action(text).unwrap(), expected);
}

#[test]
fn parse_disallowed_action_rejects_an_unknown_value() {
    assert!(parse_disallowed_action("ignore").is_err());
}
