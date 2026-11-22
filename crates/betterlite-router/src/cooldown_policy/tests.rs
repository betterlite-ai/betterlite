#![cfg(test)]

use std::future::Future;

use rstest::rstest;

mod behaviors;
mod boundaries;
mod support;

#[rstest]
#[case::strict_threshold_and_one_event(behaviors::strict_threshold_and_one_event())]
#[case::disabled_cooldowns_count_but_never_cool(
    behaviors::disabled_cooldowns_count_the_failure_but_never_cool()
)]
#[case::failing_observer_keeps_the_cooldown(
    behaviors::a_failing_observer_does_not_mask_the_cooldown()
)]
#[case::fallback_cools_the_stamped_deployment(
    behaviors::fallback_cools_the_server_stamped_deployment()
)]
#[case::generic_404_and_caller_timeout_never_cool(
    behaviors::generic_404_and_caller_timeout_never_cool()
)]
#[case::deployment_times_beat_the_header(behaviors::deployment_cooldown_times_beat_the_header())]
#[case::event_fields_come_from_the_deployment(behaviors::event_fields_come_from_the_deployment())]
#[case::base_url_alias_and_openai_default_feed_the_event(
    behaviors::base_url_alias_and_openai_default_feed_the_event()
)]
#[case::alternatives_lift_the_single_deployment_net(
    behaviors::alternatives_lift_the_single_deployment_net()
)]
#[case::provider_default_deployments_never_cool(
    behaviors::provider_default_deployments_never_cool()
)]
#[case::debug_info_lists_only_the_cooling_deployment(
    behaviors::debug_info_lists_only_the_cooling_deployment()
)]
#[case::advisor_failures_skip_both_paths(behaviors::advisor_failures_skip_both_paths())]
#[case::explicit_policy_cools_without_a_status(
    behaviors::an_explicit_policy_cools_without_a_status()
)]
#[case::team_id_without_a_public_name_does_not_lift_the_net(
    behaviors::a_team_id_without_a_public_name_does_not_lift_the_net()
)]
#[case::shared_fail_counts_answer_across_workers(
    behaviors::shared_fail_counts_answer_across_workers()
)]
#[case::unknown_deployments_never_cool(behaviors::unknown_deployments_never_cool())]
#[case::set_cools_a_known_deployment(behaviors::set_cools_a_known_deployment())]
#[tokio::test]
async fn cooldown_policy_behaviors<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    case.await;
}

#[rstest]
#[case::fail_count_is_strict_at_the_threshold(
    boundaries::the_fail_count_is_strict_at_the_threshold()
)]
#[case::zero_allowed_fails_cools_at_once(
    boundaries::zero_allowed_fails_cools_on_the_first_failure()
)]
#[case::zero_header_stops_and_a_tiny_header_keeps_it(
    boundaries::a_zero_header_stops_the_cooldown_and_a_tiny_one_keeps_it()
)]
#[case::negative_header_falls_to_the_default(
    boundaries::a_negative_header_falls_to_the_router_default()
)]
#[case::negative_model_info_hides_a_positive_params_value(
    boundaries::a_negative_model_info_value_hides_a_positive_params_value()
)]
#[case::text_status_is_cast_and_stored(boundaries::a_text_status_is_cast_and_stored())]
#[case::value_types_are_classified(boundaries::deployment_value_types_are_classified())]
#[case::ratio_minimum_requests_sits_at_five(
    boundaries::the_ratio_minimum_requests_boundary_sits_at_five()
)]
#[case::one_routable_team_sibling_keeps_the_net(
    boundaries::a_team_with_one_routable_sibling_keeps_the_net()
)]
#[tokio::test]
async fn cooldown_policy_boundaries<Fut>(#[case] case: Fut)
where
    Fut: Future<Output = ()>,
{
    case.await;
}
