#![cfg(test)]

use rstest::rstest;

use super::{CostTrackingEndUserResolver, EndUserResolver};
use crate::end_user::EndUserCostTrackingSettings;

fn resolver(disable: Option<bool>, prometheus_only: Option<bool>) -> CostTrackingEndUserResolver {
    CostTrackingEndUserResolver {
        settings: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: disable,
            enable_prometheus_only: prometheus_only,
        },
    }
}

#[rstest]
#[case::exact_true_flag(Some(true), None, Some("customer"), Some("customer"))]
#[case::false_flag(Some(false), None, Some("customer"), None)]
#[case::unset_flag(None, None, Some("customer"), None)]
#[case::disable_overrides(Some(true), Some(true), Some("customer"), None)]
#[case::no_end_user(Some(true), None, None, None)]
#[case::empty_end_user(Some(true), None, Some(""), None)]
fn the_prometheus_service_needs_the_exact_true_flag(
    #[case] prometheus_only: Option<bool>,
    #[case] disable: Option<bool>,
    #[case] end_user: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let resolved = resolver(disable, prometheus_only)
        .resolve_end_user(end_user)
        .expect("the end user id");
    assert_eq!(resolved.as_deref(), expected);
}
