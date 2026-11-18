#![cfg(test)]

mod behaviors;
mod boundaries;
mod support;

use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

fn runtime(paused: bool) -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .start_paused(paused)
        .build()
        .unwrap()
}

// the callback sink's inline flush needs the multi-threaded runtime the gateway runs on
fn worker_runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[rstest]
#[case::window_increment_returns_and_queues("window_increment_returns_and_queues", false)]
#[case::window_increment_without_a_store("window_increment_without_a_store", false)]
#[case::fractional_spend_is_kept_exactly("fractional_spend_is_kept_exactly", false)]
#[case::spend_beyond_i64_adds("spend_beyond_i64_adds", false)]
#[case::tracked_key_accessors("tracked_key_accessors", false)]
#[case::success_and_failure_counters("success_and_failure_counters", false)]
#[case::sync_merges_the_stored_value("sync_merges_the_stored_value", false)]
#[case::sync_keeps_a_value_redis_is_behind_on("sync_keeps_a_value_redis_is_behind_on", false)]
#[case::failed_push_discards_its_batch("failed_push_discards_its_batch", false)]
#[case::periodic_sync_writes_on_the_interval("periodic_sync_writes_on_the_interval", true)]
#[case::no_periodic_task_without_batching("no_periodic_task_without_batching", true)]
#[case::cleanup_stops_the_periodic_task("cleanup_stops_the_periodic_task", true)]
#[case::retire_pushes_and_stops_the_task("retire_pushes_and_stops_the_task", true)]
#[case::usage_pipeline_increments_locally_and_queues(
    "usage_pipeline_increments_locally_and_queues",
    false
)]
#[case::usage_key_merges_the_shared_store_total("usage_key_merges_the_shared_store_total", false)]
#[case::sync_compresses_repeated_keys_into_one_operation(
    "sync_compresses_repeated_keys_into_one_operation",
    false
)]
#[case::concurrent_increment_during_the_push_is_not_double_counted(
    "concurrent_increment_during_the_push_is_not_double_counted",
    false
)]
#[case::concurrent_increment_under_a_leading_store_is_merged_once(
    "concurrent_increment_under_a_leading_store_is_merged_once",
    false
)]
#[case::counter_write_and_read_round_trip("counter_write_and_read_round_trip", false)]
#[case::metadata_bucket_selection("metadata_bucket_selection", false)]
#[case::key_pattern_is_absent("key_pattern_is_absent", false)]
#[case::real_store_smoke("real_store_smoke", false)]
#[case::callback_saturation_flushes_before_dropping(
    "callback_saturation_flushes_before_dropping",
    false
)]
#[case::failure_callback_increment_survives_the_merge(
    "failure_callback_increment_survives_the_merge",
    false
)]
#[case::sync_treats_a_missing_reply_as_zero("sync_treats_a_missing_reply_as_zero", false)]
#[case::saturated_flush_past_the_deadline_drops_its_delta(
    "saturated_flush_past_the_deadline_drops_its_delta",
    true
)]
fn routing_state_behaviors(#[case] name: &str, #[case] paused: bool) {
    runtime(paused).block_on(behaviors::run(name));
}

#[rstest]
#[case::queue_cap_is_1024("queue_cap_is_1024", false)]
#[case::callback_saturation_without_a_worker_drops(
    "callback_saturation_without_a_worker_drops",
    false
)]
#[case::tracked_cap_is_1024("tracked_cap_is_1024", false)]
#[case::window_increment_rejects_an_oversized_key(
    "window_increment_rejects_an_oversized_key",
    false
)]
#[case::counter_sink_rejects_an_oversized_key("counter_sink_rejects_an_oversized_key", false)]
#[case::outcome_counter_expiry_boundaries("outcome_counter_expiry_boundaries", false)]
#[case::cancelled_sync_task_does_not_tick("cancelled_sync_task_does_not_tick", true)]
#[case::sync_without_a_store_returns_false("sync_without_a_store_returns_false", false)]
#[case::metadata_first_bucket_decides("metadata_first_bucket_decides", false)]
#[case::sync_merges_at_the_equality_boundary("sync_merges_at_the_equality_boundary", false)]
#[case::sync_interval_resolution_boundaries("sync_interval_resolution_boundaries", false)]
#[case::local_counter_reads_only_whole_values("local_counter_reads_only_whole_values", false)]
fn routing_state_boundaries(#[case] name: &str, #[case] paused: bool) {
    runtime(paused).block_on(boundaries::run(name));
}
