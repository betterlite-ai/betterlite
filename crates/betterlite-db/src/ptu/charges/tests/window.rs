use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{day, moment, ptu_model};
use crate::ptu::charges::pricing::PtuModel;
use crate::ptu::charges::window::{backfill_window, lapsed_models};

fn closing(name: &str, bound: &str) -> PtuModel {
    let mut model = ptu_model(name, name, "team-1", 1, Decimal::ONE);
    model.effective_to = Some(moment(bound));
    model
}

#[test]
fn a_window_ending_exactly_now_has_lapsed() {
    let model = closing("model-a", "2026-06-01T00:00:00Z");
    let names = lapsed_models(&[model], moment("2026-06-01T00:00:00Z"));
    assert_eq!(names, ["model-a"]);
}

#[test]
fn open_and_future_windows_have_not_lapsed() {
    let open = ptu_model("open", "open", "team-1", 1, Decimal::ONE);
    let future = closing("future", "2026-07-01T00:00:00Z");
    let names = lapsed_models(&[open, future], moment("2026-06-01T00:00:00Z"));
    assert!(names.is_empty());
}

#[test]
fn lapsed_names_are_newest_bound_first() {
    let models = [
        closing("oldest", "2026-01-01T00:00:00Z"),
        closing("newest", "2026-03-01T00:00:00Z"),
        closing("middle", "2026-02-01T00:00:00Z"),
    ];
    let names = lapsed_models(&models, moment("2026-06-01T00:00:00Z"));
    assert_eq!(names, ["newest", "middle", "oldest"]);
}

#[test]
fn equal_bounds_keep_input_order() {
    let models = [
        closing("first", "2026-02-01T00:00:00Z"),
        closing("second", "2026-02-01T00:00:00Z"),
    ];
    let names = lapsed_models(&models, moment("2026-06-01T00:00:00Z"));
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn slack_markup_is_escaped_once_in_order() {
    let models = [
        closing("a&b<c>d", "2026-02-01T00:00:00Z"),
        closing("&lt;", "2026-03-01T00:00:00Z"),
    ];
    let names = lapsed_models(&models, moment("2026-06-01T00:00:00Z"));
    assert_eq!(names, ["&amp;lt;", "a&amp;b&lt;c&gt;d"]);
}

fn starting(name: &str, from: &str) -> PtuModel {
    let mut model = ptu_model(name, name, "team-1", 1, Decimal::ONE);
    model.effective_from = moment(from);
    model
}

#[test]
fn the_floor_start_covers_ninety_one_dates() {
    let model = starting("old", "2025-01-01T00:00:00Z");
    let days = backfill_window(&[model], day("2026-04-30"));
    assert_eq!(days.len(), 91);
    assert_eq!(days[0], day("2026-01-30"));
    assert_eq!(days[90], day("2026-04-30"));
}

#[test]
fn a_start_after_the_floor_is_kept() {
    let model = starting("recent", "2026-02-01T00:00:00Z");
    let days = backfill_window(&[model], day("2026-04-30"));
    assert_eq!(days.len(), 89);
    assert_eq!(days[0], day("2026-02-01"));
    assert_eq!(days[88], day("2026-04-30"));
}

#[test]
fn the_earliest_start_wins_over_a_later_one() {
    let models = [
        starting("later", "2026-02-01T00:00:00Z"),
        starting("earliest", "2026-01-15T00:00:00Z"),
    ];
    let days = backfill_window(&models, day("2026-04-30"));
    assert_eq!(days[0], day("2026-01-30"));
}

#[test]
fn a_window_starting_on_end_covers_that_day_alone() {
    let model = starting("today", "2026-04-30T00:00:00Z");
    let days = backfill_window(&[model], day("2026-04-30"));
    assert_eq!(days, [day("2026-04-30")]);
}

#[test]
fn no_models_mean_no_dates() {
    assert!(backfill_window(&[], day("2026-04-30")).is_empty());
}

#[test]
fn future_starts_mean_no_dates() {
    let model = starting("future", "2026-05-01T00:00:00Z");
    assert!(backfill_window(&[model], day("2026-04-30")).is_empty());
}

#[test]
fn a_floor_before_the_first_date_keeps_the_earliest_start() {
    let model = starting("ancient", "0001-01-01T00:00:00Z");
    let days = backfill_window(&[model], day("0001-01-01"));
    assert_eq!(days, [day("0001-01-01")]);
    assert_eq!(days.len(), 1);
}

#[test]
fn the_first_date_of_the_era_still_reaches_a_later_end() {
    let model = starting("ancient", "0001-01-01T00:00:00Z");
    let end: NaiveDate = day("0001-01-02");
    let days = backfill_window(&[model], end);
    assert_eq!(days, [day("0001-01-01"), end]);
}
