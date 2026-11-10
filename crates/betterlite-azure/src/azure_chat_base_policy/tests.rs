#![cfg(test)]

use rstest::rstest;

mod content;
mod mapping;
mod support;

#[rstest]
#[case::api_version_response_format(mapping::api_version_response_format)]
#[case::max_completion_tokens(mapping::max_completion_tokens)]
#[case::azure_message_content(content::azure_message_content)]
#[case::sanitized_tool_schema(content::sanitized_tool_schema)]
#[case::unsupported_param(mapping::unsupported_param)]
#[case::azure_tool_choice_date_gate(mapping::azure_tool_choice_date_gate)]
#[case::azure_required_tool_choice(mapping::azure_required_tool_choice)]
#[case::azure_parameter_shapes(mapping::azure_parameter_shapes)]
#[case::azure_tool_choice_without_tools(content::azure_tool_choice_without_tools)]
#[case::azure_class_defaults(content::azure_class_defaults)]
#[case::azure_special_auth_regions(content::azure_special_auth_regions)]
#[case::azure_error_factory(content::azure_error_factory)]
fn azure_chat_base_policy_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn azure_chat_base_policy_settings_deserialize() {
    content::settings_deserialization();
}

#[test]
fn azure_chat_base_policy_response_format_is_bad_request() {
    mapping::response_format_error_class();
}

#[test]
fn azure_chat_base_policy_non_object_schema_is_bad_request() {
    mapping::non_object_schema_error_class();
}

#[test]
fn azure_chat_base_policy_tools_not_list_is_bad_request() {
    mapping::tools_not_list_error_class();
}
