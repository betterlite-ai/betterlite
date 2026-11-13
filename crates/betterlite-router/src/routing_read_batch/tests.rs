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
#[case::arm_without_a_remote_store("arm_without_a_remote_store")]
#[case::arm_keeps_the_first_prefetch("arm_keeps_the_first_prefetch")]
#[case::arm_reserves_only_missing_keys("arm_reserves_only_missing_keys")]
#[case::arm_merges_cooldown_and_usage_keys("arm_merges_cooldown_and_usage_keys")]
#[case::arm_with_nothing_due("arm_with_nothing_due")]
#[case::read_uses_the_prefetch_once("read_uses_the_prefetch_once")]
#[case::read_falls_back_after_a_failed_prefetch("read_falls_back_after_a_failed_prefetch")]
#[case::read_falls_back_when_the_prefetch_is_short("read_falls_back_when_the_prefetch_is_short")]
#[case::backfill_keeps_local_values("backfill_keeps_local_values")]
#[case::read_merges_the_usage_counters("read_merges_the_usage_counters")]
#[case::the_prefetched_usage_serves_the_next_read("the_prefetched_usage_serves_the_next_read")]
#[case::prefetched_usage_covers_the_strategy_keys("prefetched_usage_covers_the_strategy_keys")]
#[case::prefetched_usage_values_are_none_when_failed(
    "prefetched_usage_values_are_none_when_failed"
)]
#[case::read_without_a_selector_keeps_no_prefetched_usage(
    "read_without_a_selector_keeps_no_prefetched_usage"
)]
#[case::requests_are_isolated("requests_are_isolated")]
#[case::cancelled_read_releases_the_reservation("cancelled_read_releases_the_reservation")]
#[case::for_strategy_uses_usage_keys_without_a_prefetch(
    "for_strategy_uses_usage_keys_without_a_prefetch"
)]
#[case::for_strategy_needs_an_armed_prefetch_without_usage(
    "for_strategy_needs_an_armed_prefetch_without_usage"
)]
#[case::for_strategy_keeps_no_usage_keys("for_strategy_keeps_no_usage_keys")]
fn routing_read_batch_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::reservation_is_due_at_the_exact_expiry("reservation_is_due_at_the_exact_expiry")]
#[case::local_value_is_readable_at_its_exact_expiry("local_value_is_readable_at_its_exact_expiry")]
#[case::take_requires_every_needed_key("take_requires_every_needed_key")]
#[case::take_releases_a_short_prefetch("take_releases_a_short_prefetch")]
#[case::local_entries_are_capped("local_entries_are_capped")]
#[case::last_reads_are_capped("last_reads_are_capped")]
#[case::apply_prefetched_requires_a_covered_key("apply_prefetched_requires_a_covered_key")]
#[case::empty_keys_never_arm_or_read("empty_keys_never_arm_or_read")]
#[case::usage_without_keys_still_tracks_the_selector(
    "usage_without_keys_still_tracks_the_selector"
)]
#[case::circuit_open_keeps_the_local_values("circuit_open_keeps_the_local_values")]
#[case::unavailable_keeps_the_reservation("unavailable_keeps_the_reservation")]
#[case::an_absent_remote_value_still_consumes_the_prefetch(
    "an_absent_remote_value_still_consumes_the_prefetch"
)]
#[case::reservation_age_is_the_elapsed_time("reservation_age_is_the_elapsed_time")]
#[case::default_read_expiry_is_ten_seconds("default_read_expiry_is_ten_seconds")]
#[case::default_local_ttl_is_six_hundred_seconds("default_local_ttl_is_six_hundred_seconds")]
#[case::backfill_ttl_comes_from_the_tier("backfill_ttl_comes_from_the_tier")]
#[case::duplicate_cooldown_keys_are_read_once("duplicate_cooldown_keys_are_read_once")]
#[case::real_store_opens_the_circuit_and_releases("real_store_opens_the_circuit_and_releases")]
#[case::real_store_keeps_the_reservation_when_unavailable(
    "real_store_keeps_the_reservation_when_unavailable"
)]
#[case::real_store_serves_a_recovered_probe("real_store_serves_a_recovered_probe")]
fn routing_read_batch_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}
