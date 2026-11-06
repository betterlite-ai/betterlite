#![cfg(test)]

use rstest::rstest;

use super::resolved_service_tier;

#[rstest]
#[case::response_tier_is_served(Some("default"), None, None, Some("default"))]
#[case::usage_tier_is_served_when_the_response_has_none(
    None,
    Some("priority"),
    None,
    Some("priority")
)]
#[case::response_tier_wins_over_the_usage_tier(
    Some("default"),
    Some("priority"),
    Some("flex"),
    Some("default")
)]
#[case::served_tier_wins_over_the_requested_tier(None, Some("batch"), Some("auto"), Some("batch"))]
#[case::unknown_usage_tier_is_served_unrestricted(
    None,
    Some("provider-tier"),
    Some("auto"),
    Some("provider-tier")
)]
#[case::provider_tiers_are_served_unrestricted(
    Some("provider-tier"),
    None,
    None,
    Some("provider-tier")
)]
#[case::auto_requested_tier_is_known(None, None, Some("auto"), Some("auto"))]
#[case::balanced_requested_tier_is_known(None, None, Some("balanced"), Some("balanced"))]
#[case::batch_requested_tier_is_known(None, None, Some("batch"), Some("batch"))]
#[case::default_requested_tier_is_known(None, None, Some("default"), Some("default"))]
#[case::fast_requested_tier_is_known(None, None, Some("fast"), Some("fast"))]
#[case::flex_requested_tier_is_known(None, None, Some("flex"), Some("flex"))]
#[case::priority_requested_tier_is_known(None, None, Some("priority"), Some("priority"))]
#[case::scale_requested_tier_is_known(None, None, Some("scale"), Some("scale"))]
#[case::standard_requested_tier_is_known(None, None, Some("standard"), Some("standard"))]
#[case::standard_only_requested_tier_is_known(
    None,
    None,
    Some("standard_only"),
    Some("standard_only")
)]
#[case::ultrafast_requested_tier_is_known(None, None, Some("ultrafast"), Some("ultrafast"))]
#[case::unknown_requested_tier_is_dropped(None, None, Some("turbo"), None)]
#[case::uppercase_requested_tier_is_unknown(None, None, Some("AUTO"), None)]
#[case::hyphenated_requested_tier_is_unknown(None, None, Some("standard-only"), None)]
#[case::empty_requested_tier_is_unknown(None, None, Some(""), None)]
#[case::no_tier_anywhere(None, None, None, None)]
#[case::empty_served_tier_is_used_as_given(Some(""), None, Some("flex"), Some(""))]
fn parameter_projections_behavior_tests_behavior(
    #[case] response_tier: Option<&str>,
    #[case] usage_tier: Option<&str>,
    #[case] requested_tier: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        resolved_service_tier(response_tier, usage_tier, requested_tier),
        expected
    );
}
