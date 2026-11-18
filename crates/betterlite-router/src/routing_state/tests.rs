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
#[case::window_increment_returns_and_queues("window_increment_returns_and_queues")]
#[case::window_increment_without_a_store("window_increment_without_a_store")]
#[case::tracked_key_accessors("tracked_key_accessors")]
#[case::success_and_failure_counters("success_and_failure_counters")]
#[case::sync_merges_the_stored_value("sync_merges_the_stored_value")]
#[case::sync_keeps_a_value_redis_is_behind_on("sync_keeps_a_value_redis_is_behind_on")]
#[case::failed_push_discards_its_batch("failed_push_discards_its_batch")]
#[case::periodic_sync_writes_on_the_interval("periodic_sync_writes_on_the_interval")]
#[case::no_periodic_task_without_batching("no_periodic_task_without_batching")]
#[case::cleanup_stops_the_periodic_task("cleanup_stops_the_periodic_task")]
#[case::retire_pushes_and_stops_the_task("retire_pushes_and_stops_the_task")]
#[case::usage_pipeline_increments_locally_and_queues(
    "usage_pipeline_increments_locally_and_queues"
)]
#[case::counter_write_and_read_round_trip("counter_write_and_read_round_trip")]
#[case::metadata_bucket_selection("metadata_bucket_selection")]
#[case::key_pattern_is_absent("key_pattern_is_absent")]
#[case::real_store_smoke("real_store_smoke")]
fn routing_state_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::queue_cap_is_1024("queue_cap_is_1024")]
#[case::tracked_cap_is_1024("tracked_cap_is_1024")]
#[case::window_increment_rejects_an_oversized_key("window_increment_rejects_an_oversized_key")]
#[case::counter_sink_rejects_an_oversized_key("counter_sink_rejects_an_oversized_key")]
#[case::outcome_counter_expiry_boundaries("outcome_counter_expiry_boundaries")]
#[case::cancelled_sync_task_does_not_tick("cancelled_sync_task_does_not_tick")]
#[case::sync_without_a_store_returns_false("sync_without_a_store_returns_false")]
#[case::metadata_first_bucket_decides("metadata_first_bucket_decides")]
#[case::sync_merges_at_the_equality_boundary("sync_merges_at_the_equality_boundary")]
#[case::sync_interval_resolution_boundaries("sync_interval_resolution_boundaries")]
#[case::local_counter_reads_only_whole_values("local_counter_reads_only_whole_values")]
fn routing_state_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}
