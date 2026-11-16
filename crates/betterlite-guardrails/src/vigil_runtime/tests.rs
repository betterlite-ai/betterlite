#![cfg(test)]

mod build_cases;
mod cases;
mod fixtures;
mod support;
mod text_cases;
mod tool_cases;

use rstest::rstest;

use cases::Case;
use support::{run_build, run_scan};

#[rstest]
#[case::environment_defaults_fill_missing_arguments(Case::Build(Box::new(
    build_cases::environment_defaults_fill_missing_arguments()
)))]
#[case::explicit_arguments_override_environment_defaults(Case::Build(Box::new(
    build_cases::explicit_arguments_override_environment_defaults()
)))]
#[case::explicit_empty_credentials_fall_back_to_defaults(Case::Build(Box::new(
    build_cases::explicit_empty_credentials_fall_back_to_defaults()
)))]
#[case::a_long_timeout_caps_the_connect_bound(Case::Build(Box::new(
    build_cases::a_long_timeout_caps_the_connect_bound()
)))]
#[case::an_invalid_timeout_is_rejected(Case::Build(Box::new(
    build_cases::an_invalid_timeout_is_rejected()
)))]
#[case::missing_api_base_names_the_field(Case::Build(Box::new(
    build_cases::missing_api_base_names_the_field()
)))]
#[case::missing_api_key_names_the_field(Case::Build(Box::new(
    build_cases::missing_api_key_names_the_field()
)))]
#[case::explicit_supported_event_hooks_take_precedence(Case::Build(Box::new(
    build_cases::explicit_supported_event_hooks_take_precedence()
)))]
#[case::logging_only_is_accepted_for_the_apply_interface(Case::Build(Box::new(
    build_cases::logging_only_is_accepted_for_the_apply_interface()
)))]
#[case::an_unsupported_event_hook_is_rejected_in_strict_mode(Case::Build(Box::new(
    build_cases::an_unsupported_event_hook_is_rejected_in_strict_mode()
)))]
#[case::a_relaxed_strict_mode_allows_the_hook(Case::Build(Box::new(
    build_cases::a_relaxed_strict_mode_allows_the_hook()
)))]
#[case::the_fallback_value_is_matched_case_insensitively(Case::Build(Box::new(
    build_cases::the_fallback_value_is_matched_case_insensitively()
)))]
#[case::another_fallback_value_fails_closed(Case::Build(Box::new(
    build_cases::another_fallback_value_fails_closed()
)))]
#[case::no_texts_skip_the_guardrail(Case::Scan(Box::new(
    text_cases::no_texts_skip_the_guardrail()
)))]
#[case::whitespace_only_texts_skip_the_guardrail(Case::Scan(Box::new(
    text_cases::whitespace_only_texts_skip_the_guardrail()
)))]
#[case::blank_text_positions_are_kept_around_a_rewrite(Case::Scan(Box::new(
    text_cases::blank_text_positions_are_kept_around_a_rewrite()
)))]
#[case::request_tools_and_tool_calls_are_not_scanned(Case::Scan(Box::new(
    text_cases::request_tools_and_tool_calls_are_not_scanned()
)))]
#[case::a_blocked_text_stops_before_later_texts_and_tools(Case::Scan(Box::new(
    text_cases::a_blocked_text_stops_before_later_texts_and_tools()
)))]
#[case::a_text_stage_backend_failure_keeps_the_original_tool_calls(Case::Scan(Box::new(
    text_cases::a_text_stage_backend_failure_keeps_the_original_tool_calls()
)))]
#[case::a_missing_decision_fails_open_with_the_partial_texts(Case::Scan(Box::new(
    text_cases::a_missing_decision_fails_open_with_the_partial_texts()
)))]
#[case::an_unknown_decision_fails_closed(Case::Scan(Box::new(
    text_cases::an_unknown_decision_fails_closed()
)))]
#[case::an_unknown_decision_fails_open(Case::Scan(Box::new(
    text_cases::an_unknown_decision_fails_open()
)))]
#[case::metadata_and_call_id_reach_every_analysis(Case::Scan(Box::new(
    text_cases::metadata_and_call_id_reach_every_analysis()
)))]
#[case::an_empty_call_id_does_not_shadow_a_later_one(Case::Scan(Box::new(
    text_cases::an_empty_call_id_does_not_shadow_a_later_one()
)))]
#[case::an_unchanged_full_record_is_preserved(Case::Scan(Box::new(
    text_cases::an_unchanged_full_record_is_preserved()
)))]
#[case::a_sanitized_text_drops_stale_structured_messages(Case::Scan(Box::new(
    text_cases::a_sanitized_text_drops_stale_structured_messages()
)))]
#[case::an_invalid_header_fails_open(Case::Scan(Box::new(
    text_cases::an_invalid_header_fails_open()
)))]
#[case::a_non_ascii_api_key_is_not_swallowed_by_fail_open(Case::Scan(Box::new(
    text_cases::a_non_ascii_api_key_is_not_swallowed_by_fail_open()
)))]
#[case::a_non_ascii_api_key_is_not_a_backend_failure_when_closed(Case::Scan(Box::new(
    text_cases::a_non_ascii_api_key_is_not_a_backend_failure_when_closed()
)))]
#[case::an_invalid_header_fails_closed(Case::Scan(Box::new(
    text_cases::an_invalid_header_fails_closed()
)))]
#[case::an_invalid_tool_call_is_not_a_backend_failure(Case::Scan(Box::new(
    text_cases::an_invalid_tool_call_is_not_a_backend_failure()
)))]
#[case::a_non_object_answer_is_a_backend_failure(Case::Scan(Box::new(
    text_cases::a_non_object_answer_is_a_backend_failure()
)))]
#[case::a_raised_failure_carries_the_guardrail_name(Case::Scan(Box::new(
    text_cases::a_raised_failure_carries_the_guardrail_name()
)))]
#[case::texts_are_scanned_before_response_tool_arguments(Case::Scan(Box::new(
    tool_cases::texts_are_scanned_before_response_tool_arguments()
)))]
#[case::an_allowed_tool_argument_leaves_tool_calls_out(Case::Scan(Box::new(
    tool_cases::an_allowed_tool_argument_leaves_tool_calls_out()
)))]
#[case::a_tool_stage_backend_failure_keeps_the_earlier_rewrites(Case::Scan(Box::new(
    tool_cases::a_tool_stage_backend_failure_keeps_the_earlier_rewrites()
)))]
#[case::a_tool_stage_backend_failure_fails_closed(Case::Scan(Box::new(
    tool_cases::a_tool_stage_backend_failure_fails_closed()
)))]
#[case::blank_tool_arguments_are_not_scanned(Case::Scan(Box::new(
    tool_cases::blank_tool_arguments_are_not_scanned()
)))]
#[case::a_tool_stage_unknown_decision_fails_open(Case::Scan(Box::new(
    tool_cases::a_tool_stage_unknown_decision_fails_open()
)))]
#[case::a_tool_stage_unknown_decision_fails_closed(Case::Scan(Box::new(
    tool_cases::a_tool_stage_unknown_decision_fails_closed()
)))]
#[case::a_tool_only_rewrite_returns_empty_texts(Case::Scan(Box::new(
    tool_cases::a_tool_only_rewrite_returns_empty_texts()
)))]
#[case::a_blocked_tool_argument_stops_later_tool_calls(Case::Scan(Box::new(
    tool_cases::a_blocked_tool_argument_stops_later_tool_calls()
)))]
#[tokio::test]
async fn vigil_runtime_cases(#[case] case: Case) {
    match case {
        Case::Build(build) => run_build(*build),
        Case::Scan(scan) => run_scan(*scan).await,
    }
}
