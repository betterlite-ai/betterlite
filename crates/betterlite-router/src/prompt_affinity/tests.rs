#![cfg(test)]

mod behaviors;
mod boundaries;
pub(crate) mod support;

use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use crate::prompt_affinity::canonical::canonical_bytes;

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[rstest]
#[case::a_success_event_pins_the_deployment_the_next_request_reuses(
    "a_success_event_pins_the_deployment_the_next_request_reuses"
)]
#[case::a_pin_outside_the_pool_keeps_every_deployment(
    "a_pin_outside_the_pool_keeps_every_deployment"
)]
#[case::a_prompt_below_the_minimum_never_reads_the_pin(
    "a_prompt_below_the_minimum_never_reads_the_pin"
)]
#[case::a_priority_group_keeps_every_deployment("a_priority_group_keeps_every_deployment")]
#[case::a_target_order_keeps_every_deployment("a_target_order_keeps_every_deployment")]
#[case::without_messages_the_pool_is_untouched("without_messages_the_pool_is_untouched")]
#[case::another_call_type_never_pins("another_call_type_never_pins")]
#[case::a_success_event_without_a_model_id_never_pins(
    "a_success_event_without_a_model_id_never_pins"
)]
#[case::a_success_event_without_messages_never_pins("a_success_event_without_messages_never_pins")]
#[case::default_breakpoints_are_injected_when_the_global_flag_is_on(
    "default_breakpoints_are_injected_when_the_global_flag_is_on"
)]
#[case::request_kwargs_enable_prompt_caching_injects_without_the_flag(
    "request_kwargs_enable_prompt_caching_injects_without_the_flag"
)]
#[case::tools_marks_suppress_the_default_breakpoints(
    "tools_marks_suppress_the_default_breakpoints"
)]
#[case::request_kwargs_without_the_flag_do_not_inject(
    "request_kwargs_without_the_flag_do_not_inject"
)]
#[case::a_message_level_mark_ignores_later_messages("a_message_level_mark_ignores_later_messages")]
#[case::a_message_key_order_never_changes_the_digest(
    "a_message_key_order_never_changes_the_digest"
)]
#[case::only_the_blocks_up_to_the_last_mark_are_hashed(
    "only_the_blocks_up_to_the_last_mark_are_hashed"
)]
#[case::cache_control_markers_never_change_the_digest(
    "cache_control_markers_never_change_the_digest"
)]
#[case::a_typed_message_converts_to_the_dict_litellm_hashes(
    "a_typed_message_converts_to_the_dict_litellm_hashes"
)]
#[case::tools_seed_the_prefix_digest("tools_seed_the_prefix_digest")]
fn prompt_affinity_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::a_prompt_at_the_minimum_reads_the_pin("a_prompt_at_the_minimum_reads_the_pin")]
#[case::a_counting_gate_compares_the_prompt_against_the_minimum(
    "a_counting_gate_compares_the_prompt_against_the_minimum"
)]
#[case::an_unknown_model_uses_the_default_minimum("an_unknown_model_uses_the_default_minimum")]
#[case::a_deployment_without_a_model_name_uses_the_default(
    "a_deployment_without_a_model_name_uses_the_default"
)]
#[case::the_lookback_window_holds_twenty_positions("the_lookback_window_holds_twenty_positions")]
#[case::a_tool_result_run_counts_as_one_position("a_tool_result_run_counts_as_one_position")]
#[case::a_tool_result_run_in_a_user_message_counts_once(
    "a_tool_result_run_in_a_user_message_counts_once"
)]
#[case::consecutive_tool_messages_count_once("consecutive_tool_messages_count_once")]
#[case::an_empty_content_list_hashes_like_missing_content(
    "an_empty_content_list_hashes_like_missing_content"
)]
#[case::an_empty_content_string_hashes_like_missing_content(
    "an_empty_content_string_hashes_like_missing_content"
)]
#[case::a_tool_use_run_followed_by_a_tool_result_run_counts_two_positions(
    "a_tool_use_run_followed_by_a_tool_result_run_counts_two_positions"
)]
#[case::a_value_that_is_not_a_pin_is_skipped("a_value_that_is_not_a_pin_is_skipped")]
#[case::an_empty_prompt_has_no_prefix("an_empty_prompt_has_no_prefix")]
#[case::a_prompt_without_a_mark_never_pins("a_prompt_without_a_mark_never_pins")]
#[case::long_base64_payloads_hash_by_size_only("long_base64_payloads_hash_by_size_only")]
fn prompt_affinity_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}

#[test]
fn canonical_bytes_render_python_json_with_sorted_keys() {
    let value: Json = serde_json::from_str(r#"{"b":[1,true,null],"a":{"z":"x","y":2}}"#).unwrap();
    assert_eq!(
        String::from_utf8(canonical_bytes(&value)).unwrap(),
        r#"{"a":{"y":2,"z":"x"},"b":[1,true,null]}"#
    );
}
