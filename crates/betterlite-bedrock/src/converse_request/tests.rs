#![cfg(test)]

mod body_cases;
mod media;
mod request_cases;
mod support;
mod thinking_cases;

use rstest::rstest;

#[rstest]
#[case::converse_body_shape(body_cases::converse_body_shape)]
#[case::tool_config_injection(body_cases::tool_config_injection)]
#[case::context_management_filter(body_cases::context_management_filter)]
#[case::system_message_order(request_cases::system_message_order)]
#[case::inline_document_image(request_cases::inline_document_image)]
#[case::model_arn_request(request_cases::model_arn_request)]
#[case::output_format_precedence(request_cases::output_format_precedence)]
#[case::thinking_history_policy(thinking_cases::thinking_history_policy)]
fn converse_request_cases(#[case] case: fn()) {
    case();
}
