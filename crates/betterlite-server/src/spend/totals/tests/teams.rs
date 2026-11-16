use std::collections::BTreeSet;

use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal::Decimal;

use super::super::rows::TeamSpend;
use super::super::teams::{TeamTotalsError, team_totals};
use super::support::{decimal, retained};

fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

fn row(alias: Option<&str>, day: Option<&str>, spend: Option<Decimal>) -> TeamSpend {
    TeamSpend {
        team_alias: alias.map(str::to_owned),
        spend_date: day.map(date),
        total_spend: spend,
    }
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn rounds_each_daily_amount_before_summing_the_team_total() {
    let rows = [
        row(Some("alpha"), Some("2026-01-01"), Some(decimal("0.125"))),
        row(Some("alpha"), Some("2026-01-02"), Some(decimal("0.125"))),
    ];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.daily_spend[0].teams["alpha"], decimal("0.12"));
    assert_eq!(body.daily_spend[1].teams["alpha"], decimal("0.12"));
    assert_eq!(body.total_spend_per_team.len(), 1);
    assert_eq!(body.total_spend_per_team[0].total_spend, decimal("0.24"));
}

#[rstest]
#[case::binary_below_the_midpoint(2.675_f64, "2.67")]
#[case::exact_midpoint(0.125_f64, "0.12")]
#[case::binary_below_ten(10.555_f64, "10.55")]
#[case::exact_cent(1.25_f64, "1.25")]
#[case::exact_above_the_midpoint(0.135_f64, "0.14")]
fn rounds_retained_floats_to_the_nearest_even_cent(#[case] value: f64, #[case] expected: &str) {
    let rows = [row(
        Some("alpha"),
        Some("2026-01-01"),
        Some(retained(value)),
    )];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.daily_spend[0].teams["alpha"], decimal(expected));
}

#[test]
fn groups_daily_rows_by_date_and_alias() {
    let rows = [
        row(Some("beta"), Some("2026-01-02"), Some(decimal("2.00"))),
        row(Some("alpha"), Some("2026-01-01"), Some(decimal("1.50"))),
        row(Some("beta"), Some("2026-01-01"), Some(decimal("0.50"))),
    ];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.daily_spend.len(), 2);
    assert_eq!(body.daily_spend[0].date, date("2026-01-01"));
    assert_eq!(body.daily_spend[0].teams["alpha"], decimal("1.50"));
    assert_eq!(body.daily_spend[0].teams["beta"], decimal("0.50"));
    assert_eq!(body.daily_spend[1].date, date("2026-01-02"));
    assert_eq!(body.daily_spend[1].teams["beta"], decimal("2.00"));
    assert_eq!(body.teams, names(&["alpha", "beta"]));
}

#[test]
fn overwrites_the_daily_entry_and_still_adds_every_row_to_the_total() {
    let rows = [
        row(Some("alpha"), Some("2026-01-01"), Some(decimal("1.10"))),
        row(Some("alpha"), Some("2026-01-01"), Some(decimal("2.20"))),
    ];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.daily_spend[0].teams["alpha"], decimal("2.20"));
    assert_eq!(body.total_spend_per_team[0].total_spend, decimal("3.30"));
    assert_eq!(body.teams, names(&["alpha"]));
}

#[test]
fn keeps_only_the_ten_highest_team_totals() {
    let rows = [
        row(Some("a"), Some("2026-01-01"), Some(decimal("12.00"))),
        row(Some("b"), Some("2026-01-01"), Some(decimal("11.00"))),
        row(Some("c"), Some("2026-01-01"), Some(decimal("10.00"))),
        row(Some("d"), Some("2026-01-01"), Some(decimal("9.00"))),
        row(Some("e"), Some("2026-01-01"), Some(decimal("8.00"))),
        row(Some("f"), Some("2026-01-01"), Some(decimal("7.00"))),
        row(Some("g"), Some("2026-01-01"), Some(decimal("6.00"))),
        row(Some("h"), Some("2026-01-01"), Some(decimal("5.00"))),
        row(Some("i"), Some("2026-01-01"), Some(decimal("4.00"))),
        row(Some("j"), Some("2026-01-01"), Some(decimal("3.00"))),
        row(Some("k"), Some("2026-01-01"), Some(decimal("2.00"))),
        row(Some("l"), Some("2026-01-01"), Some(decimal("1.00"))),
    ];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.teams.len(), 12);
    assert_eq!(body.total_spend_per_team.len(), 10);
    assert_eq!(body.total_spend_per_team[0].team_id, "a");
    assert_eq!(body.total_spend_per_team[9].team_id, "j");
    assert!(
        !body
            .total_spend_per_team
            .iter()
            .any(|team| team.team_id == "k" || team.team_id == "l")
    );
}

#[test]
fn keeps_the_first_seen_team_first_when_totals_tie() {
    let rows = [
        row(Some("zeta"), Some("2026-01-01"), Some(decimal("1.00"))),
        row(Some("alpha"), Some("2026-01-01"), Some(decimal("1.00"))),
    ];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.total_spend_per_team[0].team_id, "zeta");
    assert_eq!(body.total_spend_per_team[1].team_id, "alpha");
}

#[test]
fn maps_a_null_alias_to_unassigned() {
    let rows = [row(None, Some("2026-01-01"), Some(decimal("0.75")))];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.teams, names(&["Unassigned"]));
    assert_eq!(body.daily_spend[0].teams["Unassigned"], decimal("0.75"));
    assert_eq!(body.total_spend_per_team[0].team_id, "Unassigned");
}

#[test]
fn skips_rows_without_a_day() {
    let rows = [
        row(Some("alpha"), None, Some(decimal("2.00"))),
        row(None, None, None),
        row(Some("beta"), Some("2026-01-01"), Some(decimal("0.25"))),
    ];
    let body = team_totals(&rows).unwrap();
    assert_eq!(body.daily_spend.len(), 1);
    assert_eq!(body.teams, names(&["beta"]));
    assert_eq!(body.total_spend_per_team.len(), 1);
    assert_eq!(body.total_spend_per_team[0].team_id, "beta");
}

#[test]
fn fails_a_row_with_a_null_total() {
    let rows = [row(Some("alpha"), Some("2026-01-01"), None)];
    assert!(matches!(
        team_totals(&rows),
        Err(TeamTotalsError::NullTotal)
    ));
}

#[test]
fn fails_when_a_team_total_overflows() {
    let rows = [
        row(Some("alpha"), Some("2026-01-01"), Some(Decimal::MAX)),
        row(Some("alpha"), Some("2026-01-02"), Some(Decimal::MAX)),
    ];
    assert!(matches!(team_totals(&rows), Err(TeamTotalsError::Overflow)));
}

#[test]
fn serializes_the_daily_amounts_as_json_numbers() {
    let rows = [row(
        Some("a"),
        Some("2026-01-01"),
        Some(retained(0.300_000_000_000_000_04)),
    )];
    let body = team_totals(&rows).unwrap();
    let encoded = serde_json::to_string(&body).unwrap();
    assert_eq!(
        encoded,
        r#"{"daily_spend":[{"date":"2026-01-01","a":0.30}],"teams":["a"],"total_spend_per_team":[{"team_id":"a","total_spend":0.30}]}"#
    );
}
