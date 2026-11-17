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
#[case::add_masks_the_exception_and_writes_both_tiers(
    "add_masks_the_exception_and_writes_both_tiers"
)]
#[case::add_uses_an_explicit_cooldown_time("add_uses_an_explicit_cooldown_time")]
#[case::read_answers_from_memory_without_a_shared_read(
    "read_answers_from_memory_without_a_shared_read"
)]
#[case::read_reads_the_shared_tier_once_for_every_due_key(
    "read_reads_the_shared_tier_once_for_every_due_key"
)]
#[case::a_warning_from_the_shared_tier_is_not_an_error(
    "a_warning_from_the_shared_tier_is_not_an_error"
)]
#[case::an_expired_cooldown_leaves_the_memory_tier("an_expired_cooldown_leaves_the_memory_tier")]
#[case::a_promoted_entry_keeps_the_remaining_time("a_promoted_entry_keeps_the_remaining_time")]
#[case::an_unexpired_local_ttl_survives_a_second_add(
    "an_unexpired_local_ttl_survives_a_second_add"
)]
#[case::a_re_add_after_the_local_ttl_expired_takes_the_new_ttl(
    "a_re_add_after_the_local_ttl_expired_takes_the_new_ttl"
)]
#[case::local_expiries_report_the_memory_tier_deadlines(
    "local_expiries_report_the_memory_tier_deadlines"
)]
#[case::set_states_keeps_an_unexpired_local_ttl("set_states_keeps_an_unexpired_local_ttl")]
#[case::the_minimum_cooldown_is_the_smallest_configured_time(
    "the_minimum_cooldown_is_the_smallest_configured_time"
)]
#[case::the_minimum_cooldown_falls_back_to_the_router_default(
    "the_minimum_cooldown_falls_back_to_the_router_default"
)]
#[case::set_states_merges_and_prunes_the_expired("set_states_merges_and_prunes_the_expired")]
#[case::set_states_prefers_the_shared_snapshot("set_states_prefers_the_shared_snapshot")]
#[case::set_states_falls_back_to_the_local_copy("set_states_falls_back_to_the_local_copy")]
#[case::a_missing_health_snapshot_marks_nothing_unhealthy(
    "a_missing_health_snapshot_marks_nothing_unhealthy"
)]
#[case::an_explicitly_healthy_state_is_not_unhealthy(
    "an_explicitly_healthy_state_is_not_unhealthy"
)]
#[case::the_sync_facade_answers_from_the_shared_snapshot(
    "the_sync_facade_answers_from_the_shared_snapshot"
)]
#[case::cooling_deployments_leave_the_pool("cooling_deployments_leave_the_pool")]
#[case::the_health_filter_is_a_no_op_until_it_is_enabled(
    "the_health_filter_is_a_no_op_until_it_is_enabled"
)]
#[case::an_allowed_fails_policy_skips_the_health_filter(
    "an_allowed_fails_policy_skips_the_health_filter"
)]
#[case::an_unhealthy_deployment_leaves_the_pool("an_unhealthy_deployment_leaves_the_pool")]
#[case::unlisted_groups_keep_their_deployments_when_scoped(
    "unlisted_groups_keep_their_deployments_when_scoped"
)]
#[case::the_health_probe_gets_an_empty_pool_when_every_candidate_is_unhealthy(
    "the_health_probe_gets_an_empty_pool_when_every_candidate_is_unhealthy"
)]
#[case::a_request_keeps_its_candidates_when_every_one_is_unhealthy(
    "a_request_keeps_its_candidates_when_every_one_is_unhealthy"
)]
#[case::a_model_with_every_deployment_paused_is_listed(
    "a_model_with_every_deployment_paused_is_listed"
)]
#[case::a_fully_unhealthy_model_is_listed_with_its_team_alias(
    "a_fully_unhealthy_model_is_listed_with_its_team_alias"
)]
#[case::an_allowed_fails_policy_hides_no_model("an_allowed_fails_policy_hides_no_model")]
fn cooldown_state_behaviors(#[case] name: &str) {
    runtime().block_on(behaviors::run(name));
}

#[rstest]
#[case::a_cooldown_ending_exactly_now_is_gone("a_cooldown_ending_exactly_now_is_gone")]
#[case::a_cooldown_ending_just_after_now_stays("a_cooldown_ending_just_after_now_stays")]
#[case::the_correction_starts_past_the_five_second_margin(
    "the_correction_starts_past_the_five_second_margin"
)]
#[case::the_corrected_ttl_is_capped_at_a_minute("the_corrected_ttl_is_capped_at_a_minute")]
#[case::the_corrected_ttl_uses_the_shorter_remaining_time(
    "the_corrected_ttl_uses_the_shorter_remaining_time"
)]
#[case::an_unhealthy_state_exactly_at_the_staleness_threshold_is_stale(
    "an_unhealthy_state_exactly_at_the_staleness_threshold_is_stale"
)]
#[case::a_state_exactly_at_the_merge_expiry_is_pruned(
    "a_state_exactly_at_the_merge_expiry_is_pruned"
)]
#[case::the_staleness_ttl_truncates_a_fractional_expiry(
    "the_staleness_ttl_truncates_a_fractional_expiry"
)]
#[case::a_key_at_the_length_limit_is_accepted_and_one_past_it_is_not(
    "a_key_at_the_length_limit_is_accepted_and_one_past_it_is_not"
)]
#[case::a_cancelled_read_leaves_the_key_readable("a_cancelled_read_leaves_the_key_readable")]
#[case::a_re_add_at_the_exact_expiry_takes_the_new_ttl(
    "a_re_add_at_the_exact_expiry_takes_the_new_ttl"
)]
#[case::set_states_replaces_a_local_ttl_ending_exactly_now(
    "set_states_replaces_a_local_ttl_ending_exactly_now"
)]
#[case::the_minimum_cooldown_of_a_zero_entry_is_zero(
    "the_minimum_cooldown_of_a_zero_entry_is_zero"
)]
fn cooldown_state_boundaries(#[case] name: &str) {
    runtime().block_on(boundaries::run(name));
}
