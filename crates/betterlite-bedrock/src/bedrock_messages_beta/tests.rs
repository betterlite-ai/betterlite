#![cfg(test)]

mod cases;
mod support;

use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_core::api::anthropic_beta_filter::error::BetaFilterError;
use rstest::rstest;

use super::beta::BetaHeadersError;

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

#[test]
fn beta_headers_error_status_codes() {
    let malformed = BetaHeadersError::Features(FeatureFlagsError::EagerInputStreaming);
    assert_eq!(malformed.status_code(), Some(400));
    let unmapped = BetaHeadersError::Filter(BetaFilterError::Field {
        field: "beta",
        expected: "a string",
    });
    assert_eq!(unmapped.status_code(), None);
}
