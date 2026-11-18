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
#[case::disabled_gate_skips_the_fit_and_placement("disabled_gate_skips_the_fit_and_placement")]
#[case::compaction_pending_skips_the_fit_and_placement(
    "compaction_pending_skips_the_fit_and_placement"
)]
#[case::no_messages_skips_the_fit_and_placement("no_messages_skips_the_fit_and_placement")]
#[case::a_full_pool_keeps_the_tier("a_full_pool_keeps_the_tier")]
#[case::a_partial_pool_restricts_the_allowed_models("a_partial_pool_restricts_the_allowed_models")]
#[case::escalation_moves_to_the_first_provable_tier("escalation_moves_to_the_first_provable_tier")]
#[case::escalation_skips_a_tier_without_proven_models(
    "escalation_skips_a_tier_without_proven_models"
)]
#[case::a_group_with_an_unknown_member_is_not_escalated_onto(
    "a_group_with_an_unknown_member_is_not_escalated_onto"
)]
#[case::unknown_windows_are_holdable_but_not_provable(
    "unknown_windows_are_holdable_but_not_provable"
)]
#[case::a_pool_override_replaces_the_tier_pool("a_pool_override_replaces_the_tier_pool")]
#[case::an_empty_pool_returns_no_placement("an_empty_pool_returns_no_placement")]
#[case::a_named_tier_can_hold_a_partial_pool("a_named_tier_can_hold_a_partial_pool")]
#[case::a_byte_bound_under_the_window_skips_the_count(
    "a_byte_bound_under_the_window_skips_the_count"
)]
#[case::a_byte_bound_over_the_window_counts("a_byte_bound_over_the_window_counts")]
#[case::the_count_covers_the_out_of_band_text("the_count_covers_the_out_of_band_text")]
#[case::out_of_band_joins_instructions_and_system("out_of_band_joins_instructions_and_system")]
#[case::body_tools_win_over_request_tools("body_tools_win_over_request_tools")]
#[case::empty_body_tools_fall_back_to_request_tools("empty_body_tools_fall_back_to_request_tools")]
#[case::a_null_or_non_text_value_contributes_nothing(
    "a_null_or_non_text_value_contributes_nothing"
)]
#[case::the_output_ceiling_is_the_smallest_limit("the_output_ceiling_is_the_smallest_limit")]
#[case::an_undeclared_output_limit_removes_the_ceiling(
    "an_undeclared_output_limit_removes_the_ceiling"
)]
#[case::a_declared_limit_beats_the_router_info("a_declared_limit_beats_the_router_info")]
#[case::an_authenticating_provider_has_no_router_limit(
    "an_authenticating_provider_has_no_router_limit"
)]
#[case::a_custom_provider_overrides_the_model_prefix(
    "a_custom_provider_overrides_the_model_prefix"
)]
#[case::catalog_limits_resolve_for_a_declared_model("catalog_limits_resolve_for_a_declared_model")]
#[case::catalog_azure_resolves_by_base_model("catalog_azure_resolves_by_base_model")]
#[case::catalog_unmapped_models_have_no_limit("catalog_unmapped_models_have_no_limit")]
#[case::message_without_content_adds_no_bytes("message_without_content_adds_no_bytes")]
#[case::message_parts_measure_as_their_json("message_parts_measure_as_their_json")]
#[case::a_match_by_name_returns_the_placement_tier("a_match_by_name_returns_the_placement_tier")]
#[case::no_placement_leaves_the_tier_and_signals("no_placement_leaves_the_tier_and_signals")]
#[case::the_fit_accepts_an_unknown_group("the_fit_accepts_an_unknown_group")]
#[case::a_disabled_gate_ignores_a_supplied_fit("a_disabled_gate_ignores_a_supplied_fit")]
#[case::catalog_custom_provider_overrides_the_model_prefix(
    "catalog_custom_provider_overrides_the_model_prefix"
)]
#[case::an_empty_parts_message_adds_no_bytes("an_empty_parts_message_adds_no_bytes")]
#[case::the_tool_truthiness_follows_python_values("the_tool_truthiness_follows_python_values")]
fn context_placement_behaviors(#[case] name: &str) {
    behaviors::run(name);
}

#[rstest]
#[case::needs_exactly_the_buffered_window("needs_exactly_the_buffered_window")]
#[case::one_token_over_the_buffered_window("one_token_over_the_buffered_window")]
#[case::the_byte_bound_equals_the_buffered_window("the_byte_bound_equals_the_buffered_window")]
#[case::the_byte_bound_is_one_over_the_buffered_window(
    "the_byte_bound_is_one_over_the_buffered_window"
)]
#[case::a_buffer_of_one_keeps_the_full_window("a_buffer_of_one_keeps_the_full_window")]
#[case::the_fit_accepts_the_exact_capacity("the_fit_accepts_the_exact_capacity")]
#[case::the_fit_rejects_one_token_over("the_fit_rejects_one_token_over")]
#[case::an_unknown_tier_outside_the_ladder_is_an_error(
    "an_unknown_tier_outside_the_ladder_is_an_error"
)]
#[case::an_unknown_tier_with_a_partial_pool_is_allowed(
    "an_unknown_tier_with_a_partial_pool_is_allowed"
)]
#[case::a_counter_failure_leaves_the_count_absent("a_counter_failure_leaves_the_count_absent")]
#[case::escalation_stops_at_the_top_tier("escalation_stops_at_the_top_tier")]
#[case::a_zero_declared_limit_is_a_limit("a_zero_declared_limit_is_a_limit")]
#[case::a_negative_declared_limit_is_not_a_window("a_negative_declared_limit_is_not_a_window")]
#[case::a_boolean_declared_limit_is_not_a_window("a_boolean_declared_limit_is_not_a_window")]
#[case::a_float_declared_limit_is_not_a_window("a_float_declared_limit_is_not_a_window")]
#[case::a_text_declared_limit_is_not_a_window("a_text_declared_limit_is_not_a_window")]
#[case::an_integer_above_u64_is_not_a_window("an_integer_above_u64_is_not_a_window")]
#[case::an_empty_group_has_unknown_facts("an_empty_group_has_unknown_facts")]
#[case::a_wildcard_group_name_has_unknown_facts("a_wildcard_group_name_has_unknown_facts")]
#[case::a_zero_output_limit_keeps_the_ceiling("a_zero_output_limit_keeps_the_ceiling")]
#[case::an_empty_default_model_is_not_a_candidate("an_empty_default_model_is_not_a_candidate")]
#[case::a_named_default_model_joins_the_facts("a_named_default_model_joins_the_facts")]
#[case::a_custom_ladder_escalates_to_a_named_tier("a_custom_ladder_escalates_to_a_named_tier")]
#[case::the_count_error_names_the_failure("the_count_error_names_the_failure")]
fn context_placement_boundaries(#[case] name: &str) {
    boundaries::run(name);
}
