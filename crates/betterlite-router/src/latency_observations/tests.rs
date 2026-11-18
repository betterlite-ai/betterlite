#![cfg(test)]

mod behaviors;
mod boundaries;
mod support;

use rstest::rstest;

#[rstest]
#[case::a_success_records_the_response_time("a_success_records_the_response_time")]
#[case::a_success_normalizes_the_response_time_by_completion_tokens(
    "a_success_normalizes_the_response_time_by_completion_tokens"
)]
#[case::a_zero_completion_token_count_keeps_the_response_time(
    "a_zero_completion_token_count_keeps_the_response_time"
)]
#[case::a_streaming_success_records_the_first_token_time(
    "a_streaming_success_records_the_first_token_time"
)]
#[case::a_streaming_success_without_a_completion_start_uses_the_end_time(
    "a_streaming_success_without_a_completion_start_uses_the_end_time"
)]
#[case::a_non_streaming_success_records_no_first_token_time(
    "a_non_streaming_success_records_no_first_token_time"
)]
#[case::the_minute_bucket_sums_tokens_and_counts_requests(
    "the_minute_bucket_sums_tokens_and_counts_requests"
)]
#[case::a_timeout_failure_appends_the_penalty("a_timeout_failure_appends_the_penalty")]
#[case::a_non_timeout_failure_records_nothing("a_non_timeout_failure_records_nothing")]
#[case::a_batch_retrieve_callback_records_nothing("a_batch_retrieve_callback_records_nothing")]
#[case::litellm_metadata_is_read_before_metadata("litellm_metadata_is_read_before_metadata")]
#[case::a_missing_model_group_or_deployment_records_nothing(
    "a_missing_model_group_or_deployment_records_nothing"
)]
#[case::the_newest_minute_starts_its_own_bucket("the_newest_minute_starts_its_own_bucket")]
#[case::an_unknown_group_has_no_samples("an_unknown_group_has_no_samples")]
fn latency_observations_behaviors(#[case] name: &str) {
    behaviors::run(name);
}

#[rstest]
#[case::a_full_window_keeps_every_sample("a_full_window_keeps_every_sample")]
#[case::the_sample_after_a_full_window_drops_the_oldest(
    "the_sample_after_a_full_window_drops_the_oldest"
)]
#[case::a_zero_or_negative_window_records_nothing("a_zero_or_negative_window_records_nothing")]
#[case::the_ttft_window_drops_its_oldest_at_the_limit(
    "the_ttft_window_drops_its_oldest_at_the_limit"
)]
#[case::a_sample_at_the_exact_expiry_keeps_the_running_window(
    "a_sample_at_the_exact_expiry_keeps_the_running_window"
)]
#[case::a_sample_one_millisecond_past_the_expiry_starts_over(
    "a_sample_one_millisecond_past_the_expiry_starts_over"
)]
#[case::a_sample_just_before_the_expiry_keeps_the_running_deadline(
    "a_sample_just_before_the_expiry_keeps_the_running_deadline"
)]
#[case::a_failure_just_past_the_expiry_starts_a_new_entry(
    "a_failure_just_past_the_expiry_starts_a_new_entry"
)]
#[case::the_penalty_window_is_bounded("the_penalty_window_is_bounded")]
#[case::a_percentile_at_the_exact_rank_takes_that_sample(
    "a_percentile_at_the_exact_rank_takes_that_sample"
)]
#[case::a_percentile_at_one_takes_the_largest_sample(
    "a_percentile_at_one_takes_the_largest_sample"
)]
#[case::an_empty_window_averages_to_zero_and_has_no_percentile(
    "an_empty_window_averages_to_zero_and_has_no_percentile"
)]
#[case::the_average_divides_the_sum_by_the_sample_count(
    "the_average_divides_the_sum_by_the_sample_count"
)]
#[case::a_non_positive_completion_count_has_no_normalized_time(
    "a_non_positive_completion_count_has_no_normalized_time"
)]
#[case::a_minute_total_past_the_integer_limit_is_an_error(
    "a_minute_total_past_the_integer_limit_is_an_error"
)]
#[case::a_model_group_past_the_key_limit_is_an_error(
    "a_model_group_past_the_key_limit_is_an_error"
)]
#[case::a_negative_clock_is_an_error("a_negative_clock_is_an_error")]
fn latency_observations_boundaries(#[case] name: &str) {
    boundaries::run(name);
}
