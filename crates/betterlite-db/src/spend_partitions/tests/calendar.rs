use chrono::NaiveDate;
use rstest::rstest;

use crate::spend_partitions::calendar::{PartitionInterval, PlannedPartition, upcoming_partitions};

fn day_of(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("a calendar date")
}

fn partition(name: &str, lower: &str, upper: &str) -> PlannedPartition {
    PlannedPartition {
        name: name.to_owned(),
        lower: NaiveDate::parse_from_str(lower, "%Y-%m-%d").expect("a lower bound"),
        upper: NaiveDate::parse_from_str(upper, "%Y-%m-%d").expect("an upper bound"),
    }
}

fn partitions(entries: &[(&str, &str, &str)]) -> Vec<PlannedPartition> {
    entries
        .iter()
        .map(|&(name, lower, upper)| partition(name, lower, upper))
        .collect()
}

#[rstest]
#[case::monday(28, "LiteLLM_SpendLogs_p20261228", "2026-12-28", "2027-01-04")]
#[case::sunday(6, "LiteLLM_SpendLogs_p20261130", "2026-11-30", "2026-12-07")]
#[case::tuesday(1, "LiteLLM_SpendLogs_p20261130", "2026-11-30", "2026-12-07")]
fn weeks_start_on_the_monday_and_cross_the_year(
    #[case] day: u32,
    #[case] name: &str,
    #[case] lower: &str,
    #[case] upper: &str,
) {
    let planned = upcoming_partitions(day_of(2026, 12, day), PartitionInterval::Week, 0);
    assert_eq!(planned, vec![partition(name, lower, upper)]);
}

#[rstest]
#[case::first_of_month(2026, 12, 1, 0, &[
    ("LiteLLM_SpendLogs_p20261201", "2026-12-01", "2027-01-01"),
])]
#[case::december(2026, 12, 15, 1, &[
    ("LiteLLM_SpendLogs_p20261201", "2026-12-01", "2027-01-01"),
    ("LiteLLM_SpendLogs_p20270101", "2027-01-01", "2027-02-01"),
])]
#[case::february(2027, 2, 10, 1, &[
    ("LiteLLM_SpendLogs_p20270201", "2027-02-01", "2027-03-01"),
    ("LiteLLM_SpendLogs_p20270301", "2027-03-01", "2027-04-01"),
])]
fn months_start_on_the_first_and_roll_over_december(
    #[case] year: i32,
    #[case] month: u32,
    #[case] day: u32,
    #[case] ahead: i64,
    #[case] expected: &[(&str, &str, &str)],
) {
    assert_eq!(
        upcoming_partitions(day_of(year, month, day), PartitionInterval::Month, ahead),
        partitions(expected),
    );
}

#[rstest]
#[case::today(2026, 12, 31, 0, &[
    ("LiteLLM_SpendLogs_p20261231", "2026-12-31", "2027-01-01"),
])]
#[case::three_days(2026, 12, 31, 2, &[
    ("LiteLLM_SpendLogs_p20261231", "2026-12-31", "2027-01-01"),
    ("LiteLLM_SpendLogs_p20270101", "2027-01-01", "2027-01-02"),
    ("LiteLLM_SpendLogs_p20270102", "2027-01-02", "2027-01-03"),
])]
#[case::leap_day(2028, 2, 28, 1, &[
    ("LiteLLM_SpendLogs_p20280228", "2028-02-28", "2028-02-29"),
    ("LiteLLM_SpendLogs_p20280229", "2028-02-29", "2028-03-01"),
])]
#[case::negative_ahead(2026, 6, 2, -1, &[])]
fn days_cover_today_and_each_ahead_day(
    #[case] year: i32,
    #[case] month: u32,
    #[case] day: u32,
    #[case] ahead: i64,
    #[case] expected: &[(&str, &str, &str)],
) {
    assert_eq!(
        upcoming_partitions(day_of(year, month, day), PartitionInterval::Day, ahead),
        partitions(expected),
    );
}

#[test]
fn interval_names_are_the_litellm_settings_values() {
    assert_eq!(
        PartitionInterval::from_name("day"),
        Some(PartitionInterval::Day)
    );
    assert_eq!(
        PartitionInterval::from_name("week"),
        Some(PartitionInterval::Week)
    );
    assert_eq!(
        PartitionInterval::from_name("month"),
        Some(PartitionInterval::Month)
    );
    assert_eq!(PartitionInterval::from_name("Day"), None);
    assert_eq!(PartitionInterval::from_name("weekly"), None);
    assert_eq!(PartitionInterval::from_name(""), None);
}
