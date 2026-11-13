#![cfg(test)]

mod behaviors;
mod boundaries;
mod support;

use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[rstest]
#[case::keeps_a_closed_tool_pair("keeps_a_closed_tool_pair")]
#[case::keeps_block_tool_pairs("keeps_block_tool_pairs")]
#[case::keeps_responses_function_pairs("keeps_responses_function_pairs")]
#[case::treats_mid_conversation_instructions_as_unavailable(
    "treats_mid_conversation_instructions_as_unavailable"
)]
#[case::treats_a_text_input_as_unavailable("treats_a_text_input_as_unavailable")]
#[case::treats_an_open_tool_call_as_unavailable("treats_an_open_tool_call_as_unavailable")]
#[case::treats_an_unclosed_tool_result_as_unavailable(
    "treats_an_unclosed_tool_result_as_unavailable"
)]
#[case::treats_an_empty_tool_id_as_unavailable("treats_an_empty_tool_id_as_unavailable")]
#[case::treats_duplicate_tool_calls_as_unavailable("treats_duplicate_tool_calls_as_unavailable")]
#[case::treats_an_empty_prefix_as_unavailable("treats_an_empty_prefix_as_unavailable")]
#[case::treats_unknown_responses_items_as_unavailable(
    "treats_unknown_responses_items_as_unavailable"
)]
#[case::treats_non_text_responses_blocks_as_unavailable(
    "treats_non_text_responses_blocks_as_unavailable"
)]
#[case::treats_non_function_tools_as_unavailable("treats_non_function_tools_as_unavailable")]
#[case::treats_thinking_blocks_as_unavailable("treats_thinking_blocks_as_unavailable")]
#[case::keeps_empty_thinking_blocks("keeps_empty_thinking_blocks")]
#[case::treats_missing_responses_type_as_message("treats_missing_responses_type_as_message")]
#[case::budget_prefers_max_completion_tokens("budget_prefers_max_completion_tokens")]
#[case::budget_prefers_max_output_tokens("budget_prefers_max_output_tokens")]
#[case::budget_prefers_payload_max_tokens("budget_prefers_payload_max_tokens")]
#[case::budget_falls_back_to_model_output("budget_falls_back_to_model_output")]
#[case::budget_has_no_available_without_window("budget_has_no_available_without_window")]
#[case::budget_has_no_available_without_output("budget_has_no_available_without_output")]
#[case::budget_rejects_a_zero_output_allowance("budget_rejects_a_zero_output_allowance")]
#[case::budget_rejects_a_negative_output_allowance("budget_rejects_a_negative_output_allowance")]
#[case::compactor_input_keeps_tools_system_and_user("compactor_input_keeps_tools_system_and_user")]
#[case::selects_the_configured_group("selects_the_configured_group")]
#[case::selects_the_first_eligible_group("selects_the_first_eligible_group")]
#[case::skips_a_group_without_a_provider("skips_a_group_without_a_provider")]
#[case::skips_a_group_that_lacks_support("skips_a_group_that_lacks_support")]
#[case::skips_a_group_with_incompatible_defaults("skips_a_group_with_incompatible_defaults")]
#[case::skips_a_group_that_cannot_hold_the_summary("skips_a_group_that_cannot_hold_the_summary")]
#[case::responses_compactor_input_uses_the_older_history(
    "responses_compactor_input_uses_the_older_history"
)]
#[case::counts_the_summary_request("counts_the_summary_request")]
#[case::propagates_a_counter_failure("propagates_a_counter_failure")]
fn compaction_history_behaviors(#[case] name: &str) {
    behaviors::run(name);
}

#[rstest]
#[case::selects_at_exactly_the_budget("selects_at_exactly_the_budget")]
#[case::skips_one_token_over_the_budget("skips_one_token_over_the_budget")]
#[case::truncates_the_window_ratio("truncates_the_window_ratio")]
#[case::accepts_an_output_allowance_of_one("accepts_an_output_allowance_of_one")]
#[case::ratio_of_one_keeps_the_window("ratio_of_one_keeps_the_window")]
#[case::negative_available_is_not_clamped("negative_available_is_not_clamped")]
#[case::error_renders_the_reason("error_renders_the_reason")]
#[case::unavailable_error_fields("unavailable_error_fields")]
#[case::rejected_error_fields("rejected_error_fields")]
#[case::split_skips_a_user_message_holding_a_tool_result(
    "split_skips_a_user_message_holding_a_tool_result"
)]
#[case::an_empty_tool_result_id_is_unavailable("an_empty_tool_result_id_is_unavailable")]
#[case::duplicate_results_without_calls_are_unavailable(
    "duplicate_results_without_calls_are_unavailable"
)]
#[case::any_unsupported_deployment_skips_the_group("any_unsupported_deployment_skips_the_group")]
#[case::only_the_first_deployment_provider_counts("only_the_first_deployment_provider_counts")]
#[case::every_deployment_budget_is_checked("every_deployment_budget_is_checked")]
#[case::configured_model_absent_from_candidates("configured_model_absent_from_candidates")]
#[case::compactor_input_slices_the_older_messages("compactor_input_slices_the_older_messages")]
#[case::call_ids_are_sorted_not_order_bound("call_ids_are_sorted_not_order_bound")]
#[case::keeps_an_open_tail_tool_call("keeps_an_open_tail_tool_call")]
#[case::counts_numeric_thinking_blocks_by_truthiness(
    "counts_numeric_thinking_blocks_by_truthiness"
)]
#[case::counts_text_and_object_thinking_blocks_by_truthiness(
    "counts_text_and_object_thinking_blocks_by_truthiness"
)]
fn compaction_history_boundaries(#[case] name: &str) {
    boundaries::run(name);
}
