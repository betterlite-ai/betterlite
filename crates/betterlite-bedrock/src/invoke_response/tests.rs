#![cfg(test)]

mod cases;
mod corpus;
mod counter;
mod run;
mod support;

use rstest::rstest;

#[rstest]
#[case::base_family_outputs(cases::base_family_outputs::base_family_outputs)]
#[case::family_reply_delegation(cases::family_reply_delegation::family_reply_delegation)]
#[case::decode_and_processing_errors(
    cases::decode_and_processing_errors::decode_and_processing_errors
)]
#[case::content_assignment_and_empty(
    cases::content_assignment_and_empty::content_assignment_and_empty
)]
#[case::response_header_token_counts(
    cases::response_header_token_counts::response_header_token_counts
)]
fn invoke_response_cases(#[case] case: fn()) {
    case();
}
