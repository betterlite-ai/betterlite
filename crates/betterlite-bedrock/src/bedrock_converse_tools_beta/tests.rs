#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::anthropic_beta_fields(cases::anthropic_beta_fields)]
#[case::tool_name_map(cases::tool_name_map)]
#[case::output_config_tools(cases::output_config_tools)]
#[case::computer_use_beta_versions(cases::computer_use_beta_versions)]
#[case::context_management_filter(cases::context_management_filter)]
fn bedrock_converse_tools_beta_cases(#[case] case: fn()) {
    case();
}
