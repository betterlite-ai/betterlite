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
#[case::rpm_allows_exactly_the_configured_requests("rpm_allows_exactly_the_configured_requests")]
#[case::rpm_zero_limit_counts_the_first_request("rpm_zero_limit_counts_the_first_request")]
#[case::rpm_without_a_limit_never_rejects("rpm_without_a_limit_never_rejects")]
#[case::rpm_limit_prefers_the_deployment_value("rpm_limit_prefers_the_deployment_value")]
#[case::rpm_limit_falls_back_in_order("rpm_limit_falls_back_in_order")]
#[case::rpm_window_follows_the_minute("rpm_window_follows_the_minute")]
#[case::pre_call_keeps_counting_when_the_store_is_down(
    "pre_call_keeps_counting_when_the_store_is_down"
)]
#[case::success_counts_each_callback_once("success_counts_each_callback_once")]
#[case::success_skips_batch_retrieves("success_skips_batch_retrieves")]
#[case::success_skips_an_incomplete_payload("success_skips_an_incomplete_payload")]
#[case::success_sync_and_async_share_the_counter("success_sync_and_async_share_the_counter")]
#[case::success_storage_error_is_swallowed("success_storage_error_is_swallowed")]
fn usage_v2_accounting_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::local_boundary_rejects_the_count_at_the_limit(
    "local_boundary_rejects_the_count_at_the_limit"
)]
#[case::sync_facade_uses_the_same_local_boundary("sync_facade_uses_the_same_local_boundary")]
#[case::async_rejection_carries_the_retry_header("async_rejection_carries_the_retry_header")]
#[case::success_zero_tokens_still_writes_the_key("success_zero_tokens_still_writes_the_key")]
#[case::window_ttl_expires_the_rpm_key("window_ttl_expires_the_rpm_key")]
#[case::shared_counts_do_not_decide_the_local_boundary(
    "shared_counts_do_not_decide_the_local_boundary"
)]
#[case::rpm_storage_error_fails_open("rpm_storage_error_fails_open")]
#[case::counter_loop_uses_the_v2_interval_and_batching(
    "counter_loop_uses_the_v2_interval_and_batching"
)]
#[case::the_v2_counter_loop_starts_and_stops("the_v2_counter_loop_starts_and_stops")]
#[case::sync_pre_call_counts_the_request_before_rejecting_it(
    "sync_pre_call_counts_the_request_before_rejecting_it"
)]
#[case::configured_ttl_expires_every_window_key("configured_ttl_expires_every_window_key")]
fn usage_v2_accounting_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}
