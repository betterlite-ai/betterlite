#![cfg(test)]

mod cases;
mod guards;
mod support;

use rstest::rstest;

#[rstest]
#[case::messages_invoke_url(cases::messages_invoke_url)]
#[case::messages_native_body(cases::messages_native_body)]
#[case::anthropic_beta_body(cases::anthropic_beta_body)]
#[case::aws_event_to_sse(cases::aws_event_to_sse)]
#[case::native_error_event(cases::native_error_event)]
#[case::message_start_event(cases::message_start_event)]
#[case::adaptive_display_updates(cases::adaptive_display_updates)]
#[case::unsupported_output_config_warn_drop(cases::unsupported_output_config_warn_drop)]
#[case::normalization_order(cases::normalization_order)]
#[case::error_header_projection(cases::error_header_projection)]
#[case::messages_constructor(cases::messages_constructor)]
fn messages_cases(#[case] case: fn()) {
    case();
}
