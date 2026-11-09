#![cfg(test)]

mod behaviors;
mod boundaries;
mod fake_redis;
mod support;

use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[rstest]
#[case::typed_values_round_trip("typed_values_round_trip")]
#[case::namespace_prefixes_every_key("namespace_prefixes_every_key")]
#[case::batch_get_keeps_order_and_duplicates("batch_get_keeps_order_and_duplicates")]
#[case::batch_get_empty_is_empty("batch_get_empty_is_empty")]
#[case::increment_counter_from_missing_and_existing("increment_counter_from_missing_and_existing")]
#[case::increment_spend_is_exact("increment_spend_is_exact")]
#[case::float_increments_and_pipeline("float_increments_and_pipeline")]
#[case::empty_float_pipeline_is_none("empty_float_pipeline_is_none")]
#[case::compare_and_set_first_writer("compare_and_set_first_writer")]
#[case::compare_and_set_matching_value_swaps("compare_and_set_matching_value_swaps")]
#[case::ttl_read_and_refresh("ttl_read_and_refresh")]
#[case::transport_failure_falls_back_for_reads("transport_failure_falls_back_for_reads")]
#[case::transport_failure_fails_increments("transport_failure_fails_increments")]
#[case::breaker_rejects_after_a_swallowed_failure("breaker_rejects_after_a_swallowed_failure")]
#[case::breaker_recovers_after_the_timeout("breaker_recovers_after_the_timeout")]
#[case::attach_if_missing_keeps_the_first_store("attach_if_missing_keeps_the_first_store")]
#[case::config_mapping_defaults("config_mapping_defaults")]
#[case::credentials_stay_out_of_errors("credentials_stay_out_of_errors")]
fn routing_state_store_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::ttl_zero_is_rejected("ttl_zero_is_rejected")]
#[case::ttl_negative_is_rejected("ttl_negative_is_rejected")]
#[case::ttl_infinite_is_rejected("ttl_infinite_is_rejected")]
#[case::ttl_nan_is_rejected("ttl_nan_is_rejected")]
#[case::fractional_ttl_truncates("fractional_ttl_truncates")]
#[case::oversized_compare_and_set_does_not_write("oversized_compare_and_set_does_not_write")]
#[case::route_key_length_boundary("route_key_length_boundary")]
#[case::invalid_stored_value_reads_as_missing("invalid_stored_value_reads_as_missing")]
#[case::invalid_stored_value_fails_an_increment("invalid_stored_value_fails_an_increment")]
#[case::batch_decode_failure_misses_every_key("batch_decode_failure_misses_every_key")]
#[case::counter_overflow_is_reported("counter_overflow_is_reported")]
#[case::spend_overflow_is_reported("spend_overflow_is_reported")]
#[case::increment_mismatched_kind_is_not_incrementable(
    "increment_mismatched_kind_is_not_incrementable"
)]
#[case::increment_keeps_a_live_expiry("increment_keeps_a_live_expiry")]
#[case::increment_adds_expiry_when_none("increment_adds_expiry_when_none")]
#[case::increment_without_ttl_keeps_no_expiry("increment_without_ttl_keeps_no_expiry")]
#[case::get_ttl_maps_absent_and_unexpiring_to_none("get_ttl_maps_absent_and_unexpiring_to_none")]
#[case::float_increment_invalid_ttl_uses_the_default(
    "float_increment_invalid_ttl_uses_the_default"
)]
#[case::float_pipeline_invalid_ttl_is_an_error("float_pipeline_invalid_ttl_is_an_error")]
#[case::float_pipeline_zero_ttl_is_sent("float_pipeline_zero_ttl_is_sent")]
#[case::cancelled_compare_and_set_leaves_the_connection_usable(
    "cancelled_compare_and_set_leaves_the_connection_usable"
)]
fn routing_state_store_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}
