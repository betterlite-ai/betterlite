#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::bedrock_anthropic_version(cases::bedrock_anthropic_version)]
#[case::adaptive_effort_ceiling(cases::adaptive_effort_ceiling)]
#[case::bedrock_beta_filter(cases::bedrock_beta_filter)]
#[case::tool_search_tool_name(cases::tool_search_tool_name)]
#[case::inline_document(cases::inline_document)]
#[case::claude_response_usage(cases::claude_response_usage)]
#[case::response_format_tool_stub(cases::response_format_tool_stub)]
#[case::forced_tool_choice_removed(cases::forced_tool_choice_removed)]
#[case::remote_url_inlined(cases::remote_url_inlined)]
#[case::async_media_order(cases::async_media_order)]
fn claude_invoke_cases(#[case] case: fn()) {
    case();
}

#[test]
fn claude_invoke_media_fetch() {
    cases::media_fetch();
}

#[test]
fn claude_invoke_strips_billing_header() {
    cases::billing_header();
}
