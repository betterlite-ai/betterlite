use chrono::{NaiveDate, Utc};
use rstest::rstest;

use crate::spend_partitions::calendar::{
    PartitionClock as _, PartitionInterval, PlannedPartition, UtcPartitionClock,
    upcoming_partitions,
};

fn day_of(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a calendar date")
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
#[case::week_monday(PartitionInterval::Week, "2026-12-28", 0, &[
    ("LiteLLM_SpendLogs_p20261228", "2026-12-28", "2027-01-04"),
])]
#[case::week_sunday(PartitionInterval::Week, "2026-12-06", 0, &[
    ("LiteLLM_SpendLogs_p20261130", "2026-11-30", "2026-12-07"),
])]
#[case::week_tuesday(PartitionInterval::Week, "2026-12-01", 0, &[
    ("LiteLLM_SpendLogs_p20261130", "2026-11-30", "2026-12-07"),
])]
#[case::month_first(PartitionInterval::Month, "2026-12-01", 0, &[
    ("LiteLLM_SpendLogs_p20261201", "2026-12-01", "2027-01-01"),
])]
#[case::month_december(PartitionInterval::Month, "2026-12-15", 1, &[
    ("LiteLLM_SpendLogs_p20261201", "2026-12-01", "2027-01-01"),
    ("LiteLLM_SpendLogs_p20270101", "2027-01-01", "2027-02-01"),
])]
#[case::month_february(PartitionInterval::Month, "2027-02-10", 1, &[
    ("LiteLLM_SpendLogs_p20270201", "2027-02-01", "2027-03-01"),
    ("LiteLLM_SpendLogs_p20270301", "2027-03-01", "2027-04-01"),
])]
#[case::day_today(PartitionInterval::Day, "2026-12-31", 0, &[
    ("LiteLLM_SpendLogs_p20261231", "2026-12-31", "2027-01-01"),
])]
#[case::day_three_days(PartitionInterval::Day, "2026-12-31", 2, &[
    ("LiteLLM_SpendLogs_p20261231", "2026-12-31", "2027-01-01"),
    ("LiteLLM_SpendLogs_p20270101", "2027-01-01", "2027-01-02"),
    ("LiteLLM_SpendLogs_p20270102", "2027-01-02", "2027-01-03"),
])]
#[case::day_leap_day(PartitionInterval::Day, "2028-02-28", 1, &[
    ("LiteLLM_SpendLogs_p20280228", "2028-02-28", "2028-02-29"),
    ("LiteLLM_SpendLogs_p20280229", "2028-02-29", "2028-03-01"),
])]
#[case::negative_ahead(PartitionInterval::Day, "2026-06-02", -1, &[])]
fn upcoming_partitions_cover_the_current_and_ahead_periods(
    #[case] interval: PartitionInterval,
    #[case] today: &str,
    #[case] ahead: i64,
    #[case] expected: &[(&str, &str, &str)],
) {
    assert_eq!(
        upcoming_partitions(day_of(today), interval, ahead),
        partitions(expected),
    );
}

#[test]
fn the_utc_clock_reports_the_current_utc_date() {
    let before = Utc::now().date_naive();
    let today = UtcPartitionClock.today();
    let after = Utc::now().date_naive();
    assert!(
        today == before || today == after,
        "{today} outside {before}..={after}"
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
