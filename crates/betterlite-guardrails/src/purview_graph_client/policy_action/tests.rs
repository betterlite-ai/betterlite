#![cfg(test)]

use rstest::rstest;

use super::should_block;
use crate::purview_graph_client::wire::ProcessContentResponse;

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
#[case::null_fields_count_as_empty(
    r#"{"policyActions": [{"@odata.type": null, "action": null, "restrictionAction": null}, {"action": "restrictAccess", "restrictionAction": "block"}]}"#,
    true
)]
#[case::unrelated_fields_are_ignored(
    r#"{"id": "x", "policyActions": [{"action": "restrictAccess", "restrictionAction": "block", "extra": 1}]}"#,
    true
)]
fn should_block_finds_a_blocking_restriction(#[case] raw: &str, #[case] expected: bool) {
    let response: ProcessContentResponse = serde_json::from_str(raw).unwrap();

    assert_eq!(should_block(&response), expected);
}
