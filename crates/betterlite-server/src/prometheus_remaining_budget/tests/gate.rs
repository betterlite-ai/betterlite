use std::time::Duration;

use rstest::rstest;

use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

use super::support::{RealGauge, Recorder, collectors_with, metadata, request};

#[rstest]
#[case::all_noop(&[], 0)]
#[case::team_enabled(&[RealGauge::Team], 5)]
#[case::key_enabled(&[RealGauge::Key], 5)]
#[case::user_enabled(&[RealGauge::User], 5)]
#[case::org_enabled(&[RealGauge::Org], 5)]
#[case::customer_remaining(&[RealGauge::CustomerRemaining], 5)]
#[case::customer_max_only(&[RealGauge::CustomerMax], 5)]
#[case::customer_hours_only(&[RealGauge::CustomerHours], 5)]
#[case::team_and_org(&[RealGauge::Team, RealGauge::Org], 5)]
#[tokio::test]
async fn skips_only_when_every_gauge_group_is_noop(
    #[case] real: &[RealGauge],
    #[case] lookups: usize,
) {
    let recorder = Recorder::recording();
    let fan_out = RemainingBudgetFanOut::new(
        recorder.clone(),
        Duration::from_secs(5),
        &collectors_with(real),
    );
    fan_out
        .increment_remaining_budget_metrics(request(&metadata(&[])))
        .await;
    assert_eq!(recorder.calls().len(), lookups);
    assert_eq!(recorder.finished().len(), lookups);
}
