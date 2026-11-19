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
#[case::lowest_absolute_tpm_wins("lowest_absolute_tpm_wins")]
#[case::a_tie_keeps_pool_order("a_tie_keeps_pool_order")]
#[case::a_missing_counter_counts_as_zero("a_missing_counter_counts_as_zero")]
#[case::an_explicit_zero_limit_admits_no_tokens("an_explicit_zero_limit_admits_no_tokens")]
#[case::the_tpm_boundary_excludes_the_sum_past_the_limit(
    "the_tpm_boundary_excludes_the_sum_past_the_limit"
)]
#[case::the_rpm_boundary_rejects_the_request_that_reaches_it(
    "the_rpm_boundary_rejects_the_request_that_reaches_it"
)]
#[case::an_unread_rpm_counter_does_not_screen("an_unread_rpm_counter_does_not_screen")]
#[case::the_limit_folds_from_the_deployment_first("the_limit_folds_from_the_deployment_first")]
#[case::the_limit_folds_through_litellm_params("the_limit_folds_through_litellm_params")]
#[case::a_null_limit_falls_through("a_null_limit_falls_through")]
#[case::a_limit_from_model_info_is_used("a_limit_from_model_info_is_used")]
#[case::a_repeated_id_keeps_its_place_and_its_last_deployment(
    "a_repeated_id_keeps_its_place_and_its_last_deployment"
)]
#[case::an_id_with_a_colon_keeps_its_counter("an_id_with_a_colon_keeps_its_counter")]
#[case::a_value_of_another_kind_is_unknown("a_value_of_another_kind_is_unknown")]
#[case::a_failed_read_answers_the_cold_pick("a_failed_read_answers_the_cold_pick")]
#[case::the_async_facade_reads_the_pool_in_one_batch(
    "the_async_facade_reads_the_pool_in_one_batch"
)]
#[case::the_async_facade_uses_the_request_prefetch("the_async_facade_uses_the_request_prefetch")]
#[case::the_sync_facade_reads_the_local_tier("the_sync_facade_reads_the_local_tier")]
#[case::the_sync_facade_without_a_tier_sees_no_counters(
    "the_sync_facade_without_a_tier_sees_no_counters"
)]
#[case::the_async_facade_without_a_usage_tier_sees_no_counters(
    "the_async_facade_without_a_usage_tier_sees_no_counters"
)]
#[case::the_counter_read_returns_after_the_local_entry_expires(
    "the_counter_read_returns_after_the_local_entry_expires"
)]
fn usage_routing_v2_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::no_deployment_reports_every_loaded_deployment(
    "no_deployment_reports_every_loaded_deployment"
)]
#[case::the_async_facade_reports_the_same_pool("the_async_facade_reports_the_same_pool")]
#[case::the_error_counts_an_unread_pool_as_zero("the_error_counts_an_unread_pool_as_zero")]
#[case::the_error_keeps_a_missing_value_missing("the_error_keeps_a_missing_value_missing")]
#[case::the_error_carries_the_throttling_shape("the_error_carries_the_throttling_shape")]
#[case::a_limit_that_is_not_a_number_names_its_field(
    "a_limit_that_is_not_a_number_names_its_field"
)]
#[case::the_rpm_limit_is_named_too("the_rpm_limit_is_named_too")]
#[case::a_key_past_the_store_limit_names_its_deployment(
    "a_key_past_the_store_limit_names_its_deployment"
)]
#[case::an_empty_pool_has_no_candidate("an_empty_pool_has_no_candidate")]
#[case::a_cancelled_read_leaves_the_request_clean("a_cancelled_read_leaves_the_request_clean")]
fn usage_routing_v2_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}
