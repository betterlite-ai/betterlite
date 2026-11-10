#![cfg(test)]

use rstest::rstest;

mod scenarios;

#[rstest]
#[case::api_version_response_format(scenarios::api_version_response_format)]
#[case::max_completion_tokens(scenarios::max_completion_tokens)]
#[case::azure_message_content(scenarios::azure_message_content)]
#[case::sanitized_tool_schema(scenarios::sanitized_tool_schema)]
#[case::unsupported_param(scenarios::unsupported_param)]
#[case::azure_tool_choice_date_gate(scenarios::azure_tool_choice_date_gate)]
#[case::azure_required_tool_choice(scenarios::azure_required_tool_choice)]
#[case::azure_parameter_shapes(scenarios::azure_parameter_shapes)]
#[case::azure_tool_choice_without_tools(scenarios::azure_tool_choice_without_tools)]
#[case::azure_class_defaults(scenarios::azure_class_defaults)]
#[case::azure_special_auth_regions(scenarios::azure_special_auth_regions)]
#[case::azure_error_factory(scenarios::azure_error_factory)]
fn azure_chat_base_policy_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn azure_chat_base_policy_settings_deserialize() {
    scenarios::settings_deserialization();
}
