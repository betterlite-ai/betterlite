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
#[case::limits_disabled_allows("limits_disabled_allows")]
#[case::tpm_below_limit_allows("tpm_below_limit_allows")]
#[case::tpm_at_limit_rejects("tpm_at_limit_rejects")]
#[case::tpm_remote_higher_value_rejects("tpm_remote_higher_value_rejects")]
#[case::tpm_local_at_limit_skips_remote("tpm_local_at_limit_skips_remote")]
#[case::breaker_open_falls_back_to_local("breaker_open_falls_back_to_local")]
#[case::remote_failure_allows_without_rpm("remote_failure_allows_without_rpm")]
#[case::no_remote_reads_local_only("no_remote_reads_local_only")]
#[case::rpm_increments_first_and_rejects_over_limit("rpm_increments_first_and_rejects_over_limit")]
#[case::success_event_counts_tpm_usage("success_event_counts_tpm_usage")]
#[case::success_event_skips_absent_fields("success_event_skips_absent_fields")]
#[case::failure_event_refunds("failure_event_refunds")]
#[case::io_reservation_before_the_checks("io_reservation_before_the_checks")]
#[case::coexist_warning_logs_once("coexist_warning_logs_once")]
#[case::coexist_warning_without_id_logs_every_time("coexist_warning_without_id_logs_every_time")]
#[case::sync_facade_counts_local_only("sync_facade_counts_local_only")]
#[case::sync_success_event_counts_local_only("sync_success_event_counts_local_only")]
#[case::sync_failure_event_refunds("sync_failure_event_refunds")]
#[case::rpm_rejection_carries_error_fields("rpm_rejection_carries_error_fields")]
#[case::sync_facade_tpm_rejects_at_limit("sync_facade_tpm_rejects_at_limit")]
fn model_rate_limits_behaviors(#[case] name: &str) {
    behaviors::run(name);
}

#[rstest]
#[case::tpm_one_below_limit_allows("tpm_one_below_limit_allows")]
#[case::tpm_over_limit_by_one_rejects("tpm_over_limit_by_one_rejects")]
#[case::rpm_at_limit_allows("rpm_at_limit_allows")]
#[case::rpm_over_limit_by_one_rejects("rpm_over_limit_by_one_rejects")]
#[case::zero_tpm_limit_rejects_zero_usage("zero_tpm_limit_rejects_zero_usage")]
#[case::negative_tpm_limit_rejects_zero_usage("negative_tpm_limit_rejects_zero_usage")]
#[case::missing_remote_value_allows("missing_remote_value_allows")]
#[case::non_numeric_limit_names_the_field("non_numeric_limit_names_the_field")]
#[case::top_level_limit_wins("top_level_limit_wins")]
#[case::params_limit_beats_model_info("params_limit_beats_model_info")]
#[case::model_info_limit_used_last("model_info_limit_used_last")]
#[case::missing_model_id_counts_under_none("missing_model_id_counts_under_none")]
#[case::empty_model_id_builds_keys("empty_model_id_builds_keys")]
#[case::breaker_open_keeps_local_below_limit("breaker_open_keeps_local_below_limit")]
#[case::sync_facade_ignores_remote("sync_facade_ignores_remote")]
#[case::coexist_warning_needs_both_limit_kinds("coexist_warning_needs_both_limit_kinds")]
#[case::error_constants_match_litellm("error_constants_match_litellm")]
fn model_rate_limits_boundaries(#[case] name: &str) {
    boundaries::run(name);
}
