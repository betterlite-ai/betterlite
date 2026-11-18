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

#[rstest]
#[case::every_tier_spends_once("every_tier_spends_once", false)]
#[case::websocket_wrappers_spend_nothing("websocket_wrappers_spend_nothing", false)]
#[case::batch_call_types_still_spend("batch_call_types_still_spend", false)]
#[case::provider_without_a_period_spends_nothing("provider_without_a_period_spends_nothing", false)]
#[case::unconfigured_tiers_spend_nothing("unconfigured_tiers_spend_nothing", false)]
#[case::absent_model_id_names_no_deployment("absent_model_id_names_no_deployment", false)]
#[case::duplicate_tag_entries_spend_twice("duplicate_tag_entries_spend_twice", false)]
#[case::sync_flushes_then_copies("sync_flushes_then_copies", false)]
#[case::increments_during_the_flush_are_added_to_the_copy(
    "increments_during_the_flush_are_added_to_the_copy",
    false
)]
#[case::failed_flush_requeues_and_skips_the_copy("failed_flush_requeues_and_skips_the_copy", false)]
#[case::cancelled_sync_still_writes_the_flush("cancelled_sync_still_writes_the_flush", false)]
#[case::failed_read_keeps_the_local_spend("failed_read_keeps_the_local_spend", false)]
#[case::memory_only_sync_keeps_the_spend("memory_only_sync_keeps_the_spend", false)]
#[case::shutdown_flushes_the_queue("shutdown_flushes_the_queue", false)]
#[case::periodic_sync_copies_on_the_interval("periodic_sync_copies_on_the_interval", true)]
#[case::cancel_stops_the_periodic_sync("cancel_stops_the_periodic_sync", true)]
fn routing_budget_state_behaviors(#[case] name: &str, #[case] paused: bool) {
    runtime(paused).block_on(behaviors::run(name));
}

#[rstest]
#[case::zero_cost_still_opens_the_window("zero_cost_still_opens_the_window", false)]
#[case::spend_keys_cover_every_tier_and_the_sync_reads_them(
    "spend_keys_cover_every_tier_and_the_sync_reads_them",
    false
)]
#[case::shared_spend_read_decodes_values("shared_spend_read_decodes_values", false)]
#[case::shared_spend_read_fails_on_unreadable_replies(
    "shared_spend_read_fails_on_unreadable_replies",
    false
)]
#[case::copy_keeps_a_live_expiry_and_replaces_an_expired_one(
    "copy_keeps_a_live_expiry_and_replaces_an_expired_one",
    false
)]
#[case::repeated_failures_merge_the_batch_once("repeated_failures_merge_the_batch_once", false)]
#[case::increment_error_reports_the_key_and_source(
    "increment_error_reports_the_key_and_source",
    false
)]
#[case::invalid_clock_reports_the_key("invalid_clock_reports_the_key", false)]
#[case::start_without_a_runtime_starts_no_task("start_without_a_runtime_starts_no_task", false)]
#[case::sync_without_a_store_returns_true("sync_without_a_store_returns_true", false)]
#[case::second_shutdown_flushes_new_increments("second_shutdown_flushes_new_increments", false)]
#[case::shutdown_reports_the_entries_it_could_not_write(
    "shutdown_reports_the_entries_it_could_not_write",
    false
)]
fn routing_budget_state_boundaries(#[case] name: &str, #[case] paused: bool) {
    runtime(paused).block_on(boundaries::run(name));
}

// every increment logged while syncs run still reaches the store and the memory cache exactly once
#[test]
fn concurrent_syncs_keep_every_increment() {
    let runtime = Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(support::concurrent_syncs());
}
