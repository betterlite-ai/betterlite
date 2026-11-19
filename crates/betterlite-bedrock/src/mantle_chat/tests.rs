#![cfg(test)]

mod cases;
mod invariants;
mod support;

use rstest::rstest;

#[rstest]
#[case::mantle_chat_url(cases::mantle_chat_url)]
#[case::mantle_message_url(cases::mantle_message_url)]
#[case::beta_header_not_body(cases::beta_header_not_body)]
#[case::model_body_restoration(cases::model_body_restoration)]
#[case::mantle_native_sse(cases::mantle_native_sse)]
#[case::mantle_usage_fields(cases::mantle_usage_fields)]
#[case::remote_url_inlined(cases::remote_url_inlined)]
#[case::mantle_capability_params(cases::mantle_capability_params)]
fn mantle_chat_cases(#[case] case: fn()) {
    case();
}
