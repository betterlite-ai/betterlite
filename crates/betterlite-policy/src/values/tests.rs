#![cfg(test)]

use rstest::rstest;

use super::{
    ModelPatterns, Policy, PolicyAttachment, PolicyCondition, PolicyConfig, PolicyGuardrails,
    PolicyScope,
};
use crate::context::{PolicyMatchContext, ResolvedPolicy};

#[test]
fn policy_deserializes_with_empty_guardrails_by_default() {
    let policy: Policy = serde_json::from_str(r#"{"description": "base"}"#).unwrap();
    assert_eq!(policy.inherit, None);
    assert_eq!(policy.description.as_deref(), Some("base"));
    assert_eq!(policy.guardrails, PolicyGuardrails::default());
    assert!(policy.guardrails.get_add().is_empty());
    assert!(policy.guardrails.get_remove().is_empty());
    assert_eq!(policy.condition, None);
    assert_eq!(policy.pipeline, None);
}

#[test]
fn scope_deserializes_with_absent_lists() {
    let scope: PolicyScope = serde_json::from_str(r#"{"teams": ["healthcare"]}"#).unwrap();
    assert_eq!(scope.teams.as_deref(), Some(&["healthcare".to_owned()][..]));
    assert_eq!(scope.keys, None);
    assert_eq!(scope.models, None);
    assert_eq!(scope.tags, None);
}

#[test]
fn condition_deserializes_without_a_model() {
    let condition: PolicyCondition = serde_json::from_str("{}").unwrap();
    assert_eq!(condition.model, None);
    let single: PolicyCondition = serde_json::from_str(r#"{"model": "gpt-4"}"#).unwrap();
    assert_eq!(single.model, Some(ModelPatterns::One("gpt-4".to_owned())));
    let listed: PolicyCondition =
        serde_json::from_str(r#"{"model": ["gpt-4", "gpt-4o"]}"#).unwrap();
    assert_eq!(
        listed.model,
        Some(ModelPatterns::Many(vec![
            "gpt-4".to_owned(),
            "gpt-4o".to_owned()
        ]))
    );
}

#[test]
fn attachment_deserializes_with_defaults() {
    let attachment: PolicyAttachment = serde_json::from_str(r#"{"policy": "baseline"}"#).unwrap();
    assert_eq!(attachment.policy, "baseline");
    assert_eq!(attachment.scope, None);
    assert_eq!(attachment.teams, None);
    assert_eq!(attachment.keys, None);
    assert_eq!(attachment.models, None);
    assert_eq!(attachment.tags, None);
    assert_eq!(attachment.priority, None);
    assert!(!attachment.default);
}

#[test]
fn policy_types_refuse_unknown_fields() {
    serde_json::from_str::<Policy>(r#"{"unknown": true}"#).unwrap_err();
    serde_json::from_str::<PolicyScope>(r#"{"teams": [], "unknown": true}"#).unwrap_err();
    serde_json::from_str::<PolicyGuardrails>(r#"{"add": [], "unknown": true}"#).unwrap_err();
    serde_json::from_str::<PolicyCondition>(r#"{"model": "gpt-4", "unknown": true}"#).unwrap_err();
    serde_json::from_str::<PolicyAttachment>(r#"{"policy": "base", "unknown": true}"#).unwrap_err();
    serde_json::from_str::<PolicyConfig>(r#"{"policies": {}, "unknown": true}"#).unwrap_err();
    serde_json::from_str::<PolicyMatchContext>(r#"{"unknown": true}"#).unwrap_err();
    serde_json::from_str::<ResolvedPolicy>(r#"{"policy_name": "base", "unknown": true}"#)
        .unwrap_err();
}

#[rstest]
#[case::below_i32_min(-0x8000_0001_i64)]
#[case::above_i32_max(0x8000_0000_i64)]
fn attachment_rejects_priority_outside_the_i32_range(#[case] priority: i64) {
    let source = format!(r#"{{"policy": "base", "priority": {priority}}}"#);
    serde_json::from_str::<PolicyAttachment>(&source).unwrap_err();
}

#[rstest]
#[case::i32_min(i32::MIN)]
#[case::i32_max(i32::MAX)]
fn attachment_accepts_priority_at_the_i32_bounds(#[case] priority: i32) {
    let source = format!(r#"{{"policy": "base", "priority": {priority}}}"#);
    let attachment: PolicyAttachment = serde_json::from_str(&source).unwrap();
    assert_eq!(attachment.priority, Some(priority));
}

#[test]
fn accessors_treat_an_empty_list_like_an_absent_one() {
    let empty: PolicyScope =
        serde_json::from_str(r#"{"teams": [], "keys": [], "models": [], "tags": []}"#).unwrap();
    assert_eq!(empty.get_teams(), ["*"]);
    assert_eq!(empty.get_keys(), ["*"]);
    assert_eq!(empty.get_models(), ["*"]);
    assert!(empty.get_tags().is_empty());

    let absent: PolicyScope = serde_json::from_str("{}").unwrap();
    assert_eq!(absent.get_teams(), ["*"]);
    assert_eq!(absent.get_keys(), ["*"]);
    assert_eq!(absent.get_models(), ["*"]);
    assert!(absent.get_tags().is_empty());

    let guardrails: PolicyGuardrails =
        serde_json::from_str(r#"{"add": [], "remove": []}"#).unwrap();
    assert!(guardrails.get_add().is_empty());
    assert!(guardrails.get_remove().is_empty());

    let populated: PolicyScope =
        serde_json::from_str(r#"{"teams": ["healthcare"], "tags": ["prod"]}"#).unwrap();
    assert_eq!(populated.get_teams(), ["healthcare"]);
    assert_eq!(populated.get_tags(), ["prod"]);
}

#[test]
fn global_attachment_scope_has_star_lists_and_no_tags() {
    let attachment: PolicyAttachment =
        serde_json::from_str(r#"{"policy": "base", "scope": "*", "teams": ["ignored"]}"#).unwrap();
    assert!(attachment.is_global());
    let scope = attachment.to_policy_scope();
    assert_eq!(scope.get_teams(), ["*"]);
    assert_eq!(scope.get_keys(), ["*"]);
    assert_eq!(scope.get_models(), ["*"]);
    assert!(scope.get_tags().is_empty());
}

#[test]
fn non_global_attachment_scope_copies_its_lists() {
    let attachment: PolicyAttachment = serde_json::from_str(
        r#"{
            "policy": "healthcare",
            "scope": "ignored",
            "teams": ["healthcare-team"],
            "keys": ["dev-key-*"],
            "models": ["gpt-4*"],
            "tags": ["health-*"]
        }"#,
    )
    .unwrap();
    assert!(!attachment.is_global());
    let scope = attachment.to_policy_scope();
    assert_eq!(scope.get_teams(), ["healthcare-team"]);
    assert_eq!(scope.get_keys(), ["dev-key-*"]);
    assert_eq!(scope.get_models(), ["gpt-4*"]);
    assert_eq!(scope.get_tags(), ["health-*"]);
}

#[test]
fn policy_config_maps_policies_by_name() {
    let config: PolicyConfig = serde_json::from_str(
        r#"{"policies": {"base": {"guardrails": {"add": ["pii"]}}, "empty": {}}}"#,
    )
    .unwrap();
    assert_eq!(config.policies.len(), 2);
    let base = &config.policies["base"];
    assert_eq!(base.guardrails.get_add(), ["pii"]);
    let empty = &config.policies["empty"];
    assert!(empty.guardrails.get_add().is_empty());

    let defaulted: PolicyConfig = serde_json::from_str("{}").unwrap();
    assert!(defaulted.policies.is_empty());
}

#[test]
fn match_context_deserializes_with_defaults() {
    let context: PolicyMatchContext = serde_json::from_str(r#"{"model": "gpt-4"}"#).unwrap();
    assert_eq!(context.team_alias, None);
    assert_eq!(context.key_alias, None);
    assert_eq!(context.model.as_deref(), Some("gpt-4"));
    assert_eq!(context.tags, None);
}

#[test]
fn resolved_policy_defaults_its_lists() {
    let resolved: ResolvedPolicy = serde_json::from_str(r#"{"policy_name": "base"}"#).unwrap();
    assert_eq!(resolved.policy_name, "base");
    assert!(resolved.guardrails.is_empty());
    assert!(resolved.inheritance_chain.is_empty());
}
