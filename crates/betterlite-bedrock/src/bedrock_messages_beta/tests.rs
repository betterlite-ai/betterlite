#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::tool_search_beta(cases::tool_search_beta)]
#[case::context_filter(cases::context_filter)]
#[case::web_search_rejection(cases::web_search_rejection)]
#[case::unsupported_fields(cases::unsupported_fields)]
#[case::beta_provider_filter(cases::beta_provider_filter)]
#[case::provider_beta_identity(cases::provider_beta_identity)]
#[case::unsupported_user_beta_warning(cases::unsupported_user_beta_warning)]
fn bedrock_messages_beta_cases(#[case] case: fn()) {
    case();
}
