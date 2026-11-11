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
#[case::full_reservation_stashes_both_channels("full_reservation_stashes_both_channels")]
#[case::estimate_unavailable_reserves_one_token("estimate_unavailable_reserves_one_token")]
#[case::zero_estimate_reserves_one_token("zero_estimate_reserves_one_token")]
#[case::explicit_zero_output_cap_reserves_nothing("explicit_zero_output_cap_reserves_nothing")]
#[case::max_completion_tokens_used_when_max_tokens_absent(
    "max_completion_tokens_used_when_max_tokens_absent"
)]
#[case::max_output_tokens_used_when_other_caps_absent(
    "max_output_tokens_used_when_other_caps_absent"
)]
#[case::max_completion_tokens_wins_over_max_output_tokens(
    "max_completion_tokens_wins_over_max_output_tokens"
)]
#[case::max_tokens_wins_over_later_fields("max_tokens_wins_over_later_fields")]
#[case::model_cap_used_without_explicit_cap("model_cap_used_without_explicit_cap")]
#[case::unmapped_model_uses_4096_fallback("unmapped_model_uses_4096_fallback")]
#[case::counter_unavailable_still_stashes_reservation(
    "counter_unavailable_still_stashes_reservation"
)]
#[case::output_over_limit_rolls_back_input("output_over_limit_rolls_back_input")]
#[case::input_over_limit_rolls_back_input("input_over_limit_rolls_back_input")]
#[case::failing_release_still_reports_rate_limit("failing_release_still_reports_rate_limit")]
#[case::usage_excludes_cached_tokens("usage_excludes_cached_tokens")]
#[case::usage_falls_back_to_standard_logging("usage_falls_back_to_standard_logging")]
#[case::usage_completion_only_resolves("usage_completion_only_resolves")]
#[case::usage_input_output_aliases("usage_input_output_aliases")]
#[case::usage_cached_details_win("usage_cached_details_win")]
#[case::usage_total_tokens_only_is_absent("usage_total_tokens_only_is_absent")]
#[case::standard_logging_cached_metadata("standard_logging_cached_metadata")]
#[case::usage_missing_keeps_reservation("usage_missing_keeps_reservation")]
#[case::usage_prefers_response_over_standard_logging(
    "usage_prefers_response_over_standard_logging"
)]
#[case::messages_and_prompt_estimate_zero("messages_and_prompt_estimate_zero")]
#[case::tokenizer_counts_messages("tokenizer_counts_messages")]
#[case::tokenizer_counts_text("tokenizer_counts_text")]
#[case::store_counter_increments_and_reports("store_counter_increments_and_reports")]
#[case::redis_down_uses_the_local_count("redis_down_uses_the_local_count")]
#[case::local_counter_enforces_without_redis("local_counter_enforces_without_redis")]
#[case::store_counter_rejects_oversized_key("store_counter_rejects_oversized_key")]
fn io_token_reservations_behaviors(#[case] name: &str) {
    behaviors::run(name);
}

#[rstest]
#[case::channel_labels("channel_labels")]
#[case::current_equal_to_limit_passes("current_equal_to_limit_passes")]
#[case::current_over_limit_by_one_fails("current_over_limit_by_one_fails")]
#[case::otpm_failure_without_itpm_releases_nothing("otpm_failure_without_itpm_releases_nothing")]
#[case::zero_limit_one_token_fails("zero_limit_one_token_fails")]
#[case::negative_limit_always_fails("negative_limit_always_fails")]
#[case::negative_max_tokens_reserves_nothing("negative_max_tokens_reserves_nothing")]
#[case::float_max_tokens_is_an_error("float_max_tokens_is_an_error")]
#[case::text_max_tokens_is_an_error("text_max_tokens_is_an_error")]
#[case::bool_max_tokens_is_an_error("bool_max_tokens_is_an_error")]
#[case::null_max_tokens_uses_next_field("null_max_tokens_uses_next_field")]
#[case::model_cap_equal_to_limit_passes("model_cap_equal_to_limit_passes")]
#[case::model_cap_over_limit_by_one_fails("model_cap_over_limit_by_one_fails")]
#[case::provider_mismatched_model_uses_4096("provider_mismatched_model_uses_4096")]
#[case::overflowing_output_cap_still_rate_limits("overflowing_output_cap_still_rate_limits")]
#[case::mantle_region_prefix_is_stripped("mantle_region_prefix_is_stripped")]
#[case::non_region_prefix_is_not_stripped("non_region_prefix_is_not_stripped")]
#[case::missing_model_id_skips_limits("missing_model_id_skips_limits")]
#[case::missing_model_skips_limits("missing_model_skips_limits")]
#[case::no_limits_skips_everything("no_limits_skips_everything")]
#[case::empty_model_id_still_builds_keys("empty_model_id_still_builds_keys")]
#[case::minute_labels("minute_labels")]
#[case::reservation_channel_priority("reservation_channel_priority")]
#[case::channel_without_itpm_sentinel_is_skipped("channel_without_itpm_sentinel_is_skipped")]
#[case::clear_removes_forged_sentinels("clear_removes_forged_sentinels")]
#[case::stash_merges_existing_metadata("stash_merges_existing_metadata")]
#[case::stash_replaces_non_object_metadata("stash_replaces_non_object_metadata")]
#[case::billable_input_never_negative("billable_input_never_negative")]
#[case::standard_logging_zero_tokens_unresolved("standard_logging_zero_tokens_unresolved")]
#[case::cancelled_output_increment_keeps_input("cancelled_output_increment_keeps_input")]
fn io_token_reservations_boundaries(#[case] name: &str) {
    boundaries::run(name);
}
