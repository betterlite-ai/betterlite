#![cfg(test)]

use rstest::rstest;

use super::{HookSlot, ListKey};

#[rstest]
#[case::callbacks(ListKey::Callbacks, "callbacks")]
#[case::success_callback(ListKey::SuccessCallback, "success_callback")]
#[case::failure_callback(ListKey::FailureCallback, "failure_callback")]
#[case::audit_log_callbacks(ListKey::AuditLogCallbacks, "audit_log_callbacks")]
fn the_callback_lists_name_their_litellm_setting(#[case] key: ListKey, #[case] expected: &str) {
    assert_eq!(key.as_str(), expected);
}

#[rstest]
#[case::custom_auth(HookSlot::CustomAuth, "custom_auth")]
#[case::custom_key_generate(HookSlot::CustomKeyGenerate, "custom_key_generate")]
#[case::custom_key_update(HookSlot::CustomKeyUpdate, "custom_key_update")]
#[case::custom_key_policy(HookSlot::CustomKeyPolicy, "custom_key_policy")]
#[case::custom_team_metadata_validate(
    HookSlot::CustomTeamMetadataValidate,
    "custom_team_metadata_validate"
)]
#[case::custom_sso(HookSlot::CustomSso, "custom_sso")]
#[case::custom_ui_sso_sign_in_handler(
    HookSlot::CustomUiSsoSignInHandler,
    "custom_ui_sso_sign_in_handler"
)]
#[case::post_call_rules(HookSlot::PostCallRules, "post_call_rules")]
#[case::custom_validate(HookSlot::CustomValidate, "custom_validate")]
#[case::pass_through_adapter(HookSlot::PassThroughAdapter, "pass_through_adapter")]
#[case::mcp_tool_handler(HookSlot::McpToolHandler, "mcp_tool_handler")]
#[case::guardrail_class(HookSlot::GuardrailClass, "guardrail_class")]
#[case::custom_provider(HookSlot::CustomProvider, "custom_provider")]
#[case::routing_plugin(HookSlot::RoutingPlugin, "routing_plugin")]
#[case::classifier_plugin(HookSlot::ClassifierPlugin, "classifier_plugin")]
#[case::callbacks(HookSlot::Callback(ListKey::Callbacks), "callbacks")]
fn a_hook_slot_names_the_setting_it_came_from(#[case] slot: HookSlot, #[case] expected: &str) {
    assert_eq!(slot.as_str(), expected);
}
