#![cfg(test)]

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use rstest::rstest;

use crate::reload_schedule::ReloadSchedule;
use crate::reload_schedule::pod_reload_decision::pod_reload_is_due;

const REVISION: u32 = 4;

fn at() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(12, 6, 7)
        .unwrap()
        .and_utc()
}

#[rstest]
#[case::revision_mismatch(Some(6), None, 3, 0, true)]
#[case::revision_mismatch_without_a_schedule(None, None, 3, 0, true)]
#[case::no_interval_never_fires(None, Some(3_600), 4, 100_000, false)]
#[case::a_schedule_that_never_ran_fires(Some(6), None, 4, 0, true)]
#[case::data_loaded_within_the_interval_waits(Some(6), Some(3_600), 4, 18_000, false)]
#[case::a_second_before_the_boundary_waits(Some(6), Some(3_600), 4, 21_599, false)]
#[case::at_the_boundary_the_reload_fires(Some(6), Some(3_600), 4, 21_600, true)]
#[case::past_the_boundary_the_reload_fires(Some(6), Some(3_600), 4, 25_200, true)]
#[case::recent_data_waits_after_an_old_shared_run(Some(6), Some(36_000), 4, 3_600, false)]
fn pod_reload_decision_reads_the_revision_interval_and_pod_data(
    #[case] interval_hours: Option<u32>,
    #[case] last_run_seconds_ago: Option<i64>,
    #[case] pod_applied_revision: u32,
    #[case] loaded_seconds_ago: i64,
    #[case] expected: bool,
) {
    let current_time = at();
    let schedule = ReloadSchedule {
        interval_hours,
        reload_revision: REVISION,
        last_run_at: last_run_seconds_ago.map(|seconds| {
            current_time
                .checked_sub_signed(TimeDelta::seconds(seconds))
                .unwrap()
        }),
    };
    let pod_data_loaded_at = current_time
        .checked_sub_signed(TimeDelta::seconds(loaded_seconds_ago))
        .unwrap();
    assert_eq!(
        pod_reload_is_due(
            &schedule,
            pod_applied_revision,
            pod_data_loaded_at,
            current_time,
            "Model cost map"
        ),
        expected
    );
}
