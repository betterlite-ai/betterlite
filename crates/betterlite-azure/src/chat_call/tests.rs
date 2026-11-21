#![cfg(test)]

mod cases;
mod support;

use rstest::rstest;

#[rstest]
#[case::chat_deployment_request(cases::chat_deployment_request)]
#[case::azure_v1_body_model(cases::azure_v1_body_model)]
#[case::cross_deployment_auth(cases::cross_deployment_auth)]
#[case::missing_model_messages_422(cases::missing_model_messages_422)]
#[case::max_retries_default_two(cases::max_retries_default_two)]
#[case::invalid_retries_422(cases::invalid_retries_422)]
#[case::cloudflare_model_null(cases::cloudflare_model_null)]
#[case::gpt5_base_model_selection(cases::gpt5_base_model_selection)]
#[case::json_mode_projection(cases::json_mode_projection)]
#[case::output_limit_recovery(cases::output_limit_recovery)]
#[case::async_timeout_diagnostic(cases::async_timeout_diagnostic)]
#[case::string_reply_500(cases::string_reply_500)]
#[case::client_type_validation(cases::client_type_validation)]
#[case::error_status_passthrough(cases::error_status_passthrough)]
#[case::async_cancellation_passthrough(cases::async_cancellation_passthrough)]
#[case::o_series_client_delegate(cases::o_series_client_delegate)]
#[case::raw_headers_projection(cases::raw_headers_projection)]
fn chat_call_cases(#[case] case: fn()) {
    case();
}
