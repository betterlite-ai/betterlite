#![cfg(test)]

mod deployment_affinity_behaviors;
mod deployment_affinity_boundaries;
mod support;

use std::future::{Future, ready};

use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[rstest]
#[case::no_pin_keeps_pool(deployment_affinity_behaviors::no_pin_keeps_pool())]
#[case::caller_pin_selects(deployment_affinity_behaviors::caller_pin_selects())]
#[case::shared_pin_selects(deployment_affinity_behaviors::shared_pin_selects())]
#[case::session_pin_selects(deployment_affinity_behaviors::session_pin_selects())]
#[case::session_beats_caller(deployment_affinity_behaviors::session_beats_caller())]
#[case::session_pin_scoped_by_caller(deployment_affinity_behaviors::session_pin_scoped_by_caller())]
#[case::marker_enables_session(deployment_affinity_behaviors::marker_enables_session())]
#[case::responses_beats_session(deployment_affinity_behaviors::responses_beats_session())]
#[case::priority_group_keeps_pool(deployment_affinity_behaviors::priority_group_keeps_pool())]
#[case::target_order_keeps_pool(deployment_affinity_behaviors::target_order_keeps_pool())]
#[case::group_flags_enable(deployment_affinity_behaviors::group_flags_enable())]
#[case::group_entry_disables(deployment_affinity_behaviors::group_entry_disables())]
#[case::group_without_entry_uses_globals(
    deployment_affinity_behaviors::group_without_entry_uses_globals()
)]
#[case::unstable_group_keeps_pool(deployment_affinity_behaviors::unstable_group_keeps_pool())]
#[case::filtered_pin_not_revived(deployment_affinity_behaviors::filtered_pin_not_revived())]
#[case::hook_pins_caller(deployment_affinity_behaviors::hook_pins_caller())]
#[case::hook_pins_session(deployment_affinity_behaviors::hook_pins_session())]
#[case::hook_needs_deployment_model_name(
    deployment_affinity_behaviors::hook_needs_deployment_model_name()
)]
#[case::hook_needs_a_flag(deployment_affinity_behaviors::hook_needs_a_flag())]
#[case::hook_first_writer_wins(deployment_affinity_behaviors::hook_first_writer_wins())]
#[case::expired_pin_is_replaced(deployment_affinity_behaviors::expired_pin_is_replaced())]
#[case::hook_reads_metadata_model_info(
    deployment_affinity_behaviors::hook_reads_metadata_model_info()
)]
#[case::unknown_flags_warn({
    deployment_affinity_behaviors::unknown_flags_warn();
    ready(())
})]
#[case::litellm_metadata_caller_key(deployment_affinity_behaviors::litellm_metadata_caller_key())]
fn deployment_affinity_behaviors<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    runtime().block_on(case);
}

#[rstest]
#[case::empty_pool(deployment_affinity_boundaries::empty_pool())]
#[case::empty_previous_response_id(deployment_affinity_boundaries::empty_previous_response_id())]
#[case::non_text_previous_response_id(
    deployment_affinity_boundaries::non_text_previous_response_id()
)]
#[case::null_previous_response_id(deployment_affinity_boundaries::null_previous_response_id())]
#[case::previous_response_outside_pool(
    deployment_affinity_boundaries::previous_response_outside_pool()
)]
#[case::non_text_user_key(deployment_affinity_boundaries::non_text_user_key())]
#[case::user_id_identity(deployment_affinity_boundaries::user_id_identity())]
#[case::generated_session_ignored(deployment_affinity_boundaries::generated_session_ignored())]
#[case::generated_flag_not_bool(deployment_affinity_boundaries::generated_flag_not_bool())]
#[case::marker_zero_ignored(deployment_affinity_boundaries::marker_zero_ignored())]
#[case::marker_out_of_range(deployment_affinity_boundaries::marker_out_of_range())]
#[case::shared_value_not_a_pin_ignored(
    deployment_affinity_boundaries::shared_value_not_a_pin_ignored()
)]
#[case::legacy_text_pin_selects(deployment_affinity_boundaries::legacy_text_pin_selects())]
#[case::other_pin_ignored(deployment_affinity_boundaries::other_pin_ignored())]
#[case::empty_model_id_pin_ignored(deployment_affinity_boundaries::empty_model_id_pin_ignored())]
#[case::hook_session_only_with_marker(
    deployment_affinity_boundaries::hook_session_only_with_marker()
)]
#[case::hook_marker_sets_session_ttl(deployment_affinity_boundaries::hook_marker_sets_session_ttl())]
#[case::hook_user_ttl_uses_settings(deployment_affinity_boundaries::hook_user_ttl_uses_settings())]
#[case::hook_without_model_info(deployment_affinity_boundaries::hook_without_model_info())]
#[case::hook_unusable_model_id(deployment_affinity_boundaries::hook_unusable_model_id())]
fn deployment_affinity_boundaries<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    runtime().block_on(case);
}
