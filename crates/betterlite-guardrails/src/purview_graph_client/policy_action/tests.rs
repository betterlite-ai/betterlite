#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use super::should_block;

#[rstest]
#[case::no_actions_key("{}", false)]
#[case::empty_actions(r#"{"policyActions": []}"#, false)]
#[case::null_actions(r#"{"policyActions": null}"#, false)]
#[case::restrict_access_action_type_blocks(
    r##"{"policyActions": [{"@odata.type": "#microsoft.graph.restrictAccessAction", "restrictionAction": "block"}]}"##,
    true
)]
#[case::restrict_access_field_blocks(
    r#"{"policyActions": [{"action": "restrictAccess", "restrictionAction": "block"}]}"#,
    true
)]
#[case::other_restriction_does_not_block(
    r#"{"policyActions": [{"action": "restrictAccess", "restrictionAction": "warn"}]}"#,
    false
)]
#[case::missing_restriction_does_not_block(
    r##"{"policyActions": [{"@odata.type": "#microsoft.graph.restrictAccessAction"}]}"##,
    false
)]
#[case::block_on_another_action_does_not_block(
    r#"{"policyActions": [{"action": "audit", "restrictionAction": "block"}]}"#,
    false
)]
#[case::any_blocking_action_blocks(
    r##"{"policyActions": [{"action": "audit"}, {"@odata.type": "#microsoft.graph.restrictAccessAction", "restrictionAction": "block"}]}"##,
    true
)]
#[case::restriction_value_is_exact(
    r#"{"policyActions": [{"action": "restrictAccess", "restrictionAction": "Block"}]}"#,
    false
)]
#[case::entries_that_are_not_objects_are_skipped(
    r#"{"policyActions": ["block", {"action": "restrictAccess", "restrictionAction": "block"}]}"#,
    true
)]
fn should_block_finds_a_blocking_restriction(#[case] raw: &str, #[case] expected: bool) {
    let response: Object = serde_json::from_str(raw).unwrap();

    assert_eq!(should_block(&response), expected);
}
