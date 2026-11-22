#![cfg(test)]

mod behaviors;
mod boundaries;
mod support;

use std::future::{Future, ready};

use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

#[rstest]
#[case::no_pin_keeps_pool(behaviors::no_pin_keeps_pool())]
#[case::caller_pin_selects(behaviors::caller_pin_selects())]
#[case::shared_pin_selects(behaviors::shared_pin_selects())]
#[case::session_pin_selects(behaviors::session_pin_selects())]
#[case::session_beats_caller(behaviors::session_beats_caller())]
#[case::session_pin_scoped_by_caller(behaviors::session_pin_scoped_by_caller())]
#[case::marker_enables_session(behaviors::marker_enables_session())]
#[case::responses_beats_session(behaviors::responses_beats_session())]
#[case::priority_group_keeps_pool(behaviors::priority_group_keeps_pool())]
#[case::target_order_keeps_pool(behaviors::target_order_keeps_pool())]
#[case::group_flags_enable(behaviors::group_flags_enable())]
#[case::group_entry_disables(behaviors::group_entry_disables())]
#[case::group_without_entry_uses_globals(behaviors::group_without_entry_uses_globals())]
#[case::unstable_group_keeps_pool(behaviors::unstable_group_keeps_pool())]
#[case::filtered_pin_not_revived(behaviors::filtered_pin_not_revived())]
#[case::hook_pins_caller(behaviors::hook_pins_caller())]
#[case::hook_pins_session(behaviors::hook_pins_session())]
#[case::hook_scopes_the_session_pin_by_caller(behaviors::hook_scopes_the_session_pin_by_caller())]
#[case::hook_needs_deployment_model_name(behaviors::hook_needs_deployment_model_name())]
#[case::hook_needs_a_flag(behaviors::hook_needs_a_flag())]
#[case::hook_first_writer_wins(behaviors::hook_first_writer_wins())]
#[case::expired_pin_is_replaced(behaviors::expired_pin_is_replaced())]
#[case::hook_reads_metadata_model_info(behaviors::hook_reads_metadata_model_info())]
#[case::unknown_flags_warn({
    behaviors::unknown_flags_warn();
    ready(())
})]
#[case::litellm_metadata_caller_key(behaviors::litellm_metadata_caller_key())]
fn deployment_affinity_behaviors<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    runtime().block_on(case);
}

#[rstest]
#[case::empty_pool(boundaries::empty_pool())]
#[case::empty_previous_response_id(boundaries::empty_previous_response_id())]
#[case::non_text_previous_response_id(boundaries::non_text_previous_response_id())]
#[case::null_previous_response_id(boundaries::null_previous_response_id())]
#[case::previous_response_outside_pool(boundaries::previous_response_outside_pool())]
#[case::non_text_user_key(boundaries::non_text_user_key())]
#[case::user_id_identity(boundaries::user_id_identity())]
#[case::generated_session_ignored(boundaries::generated_session_ignored())]
#[case::generated_flag_not_bool(boundaries::generated_flag_not_bool())]
#[case::marker_zero_ignored(boundaries::marker_zero_ignored())]
#[case::marker_out_of_range(boundaries::marker_out_of_range())]
#[case::shared_value_not_a_pin_ignored(boundaries::shared_value_not_a_pin_ignored())]
#[case::legacy_text_pin_selects(boundaries::legacy_text_pin_selects())]
#[case::other_pin_ignored(boundaries::other_pin_ignored())]
#[case::empty_model_id_pin_ignored(boundaries::empty_model_id_pin_ignored())]
#[case::hook_session_only_with_marker(boundaries::hook_session_only_with_marker())]
#[case::hook_marker_sets_session_ttl(boundaries::hook_marker_sets_session_ttl())]
#[case::hook_user_ttl_uses_settings(boundaries::hook_user_ttl_uses_settings())]
#[case::hook_without_model_info(boundaries::hook_without_model_info())]
#[case::hook_unusable_model_id(boundaries::hook_unusable_model_id())]
#[case::over_long_model_group_keeps_pool(boundaries::over_long_model_group_keeps_pool())]
#[case::over_long_session_id_keeps_pool(boundaries::over_long_session_id_keeps_pool())]
fn deployment_affinity_boundaries<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    runtime().block_on(case);
}
