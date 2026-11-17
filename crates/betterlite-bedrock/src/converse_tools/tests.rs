#![cfg(test)]

mod cases;
mod error_cases;
mod response_format_cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::tool_choice_values(cases::tool_choice_values)]
#[case::computer_use_tool_split(cases::computer_use_tool_split)]
#[case::json_mode_tool(cases::json_mode_tool)]
#[case::native_structured_output(response_format_cases::native_structured_output)]
#[case::additional_properties_recursion(cases::additional_properties_recursion)]
#[case::unsupported_response_format(response_format_cases::unsupported_response_format)]
#[case::strict_tool_spec(cases::strict_tool_spec)]
#[case::supported_media_types(cases::supported_media_types)]
fn converse_tools_cases(#[case] case: fn()) {
    case();
}
