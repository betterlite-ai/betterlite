use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{day, moment, ptu_model};
use crate::ptu::charges::pricing::{
    ChargeError, PtuModel, active_hours_on_day, aggregate_charges, daily_flat_cost,
};

fn windowed(from: &str, to: Option<&str>) -> PtuModel {
    let mut model = ptu_model("model-a", "display", "team-1", 2, Decimal::new(5, 1));
    model.effective_from = moment(from);
    model.effective_to = to.map(moment);
    model
}

#[test]
fn a_full_day_inside_the_window_is_twenty_four_hours() {
    let model = windowed("2025-01-01T00:00:00Z", None);
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::from(24)
    );
}

#[test]
fn a_window_opening_at_23_charges_one_hour() {
    let model = windowed("2026-03-01T23:00:00Z", None);
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::ONE
    );
}

#[test]
fn a_window_opening_on_the_next_day_charges_nothing_that_day() {
    let model = windowed("2026-03-02T05:00:00Z", None);
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn a_window_closing_at_the_next_midnight_excludes_that_day() {
    let model = windowed("2026-01-01T00:00:00Z", Some("2026-03-02T00:00:00Z"));
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::from(24)
    );
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-02")).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn a_window_ending_at_the_day_start_charges_nothing() {
    let model = windowed("2026-01-01T00:00:00Z", Some("2026-03-01T00:00:00Z"));
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn an_empty_window_charges_nothing() {
    let model = windowed("2026-03-01T10:00:00Z", Some("2026-03-01T10:00:00Z"));
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn a_window_closing_before_it_opens_charges_nothing() {
    let model = windowed("2026-03-01T10:00:00Z", Some("2026-03-01T09:00:00Z"));
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        Decimal::ZERO
    );
}

#[rstest]
#[case::half_past_midnight("2026-03-01T00:30:00Z", Decimal::new(235, 1))]
#[case::morning("2026-03-01T06:00:00Z", Decimal::from(18))]
#[case::last_minutes("2026-03-01T23:52:30Z", Decimal::new(125, 3))]
fn a_window_opening_mid_day_counts_the_remaining_hours(
    #[case] from: &str,
    #[case] expected: Decimal,
) {
    let model = windowed(from, None);
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        expected
    );
}

#[rstest]
#[case::half_hour("2026-03-01T00:30:00Z", Decimal::new(5, 1))]
#[case::evening("2026-03-01T18:00:00Z", Decimal::from(18))]
fn a_window_closing_mid_day_counts_the_elapsed_hours(#[case] to: &str, #[case] expected: Decimal) {
    let model = windowed("2026-01-01T00:00:00Z", Some(to));
    assert_eq!(
        active_hours_on_day(&model, day("2026-03-01")).unwrap(),
        expected
    );
}

#[test]
fn the_daily_cost_multiplies_count_rate_and_hours() {
    let mut model = ptu_model("model-a", "display", "team-1", 4, Decimal::new(5, 1));
    model.effective_from = moment("2026-01-01T00:00:00Z");
    assert_eq!(
        daily_flat_cost(&model, day("2026-03-01")).unwrap(),
        Decimal::from(48)
    );
}

#[test]
fn a_day_outside_the_window_costs_nothing() {
    let model = windowed("2026-03-02T00:00:00Z", None);
    assert_eq!(
        daily_flat_cost(&model, day("2026-03-01")).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn an_unrepresentable_amount_is_an_error() {
    let model = ptu_model("model-a", "display", "team-1", 2, Decimal::MAX);
    let result = daily_flat_cost(&model, day("2026-03-01"));
    assert!(matches!(result, Err(ChargeError::AmountOverflow { .. })));
}

#[test]
fn the_day_after_the_last_date_is_an_error() {
    let model = windowed("2026-01-01T00:00:00Z", None);
    let result = active_hours_on_day(&model, day("9999-12-31"));
    assert!(matches!(result, Err(ChargeError::DayOverflow { .. })));
}

#[test]
fn charges_are_sorted_by_team_then_model() {
    let models = [
        ptu_model("model-a", "a", "team-b", 1, Decimal::ONE),
        ptu_model("model-z", "z", "team-a", 1, Decimal::ONE),
        ptu_model("model-a", "a", "team-a", 1, Decimal::ONE),
    ];
    let charges = aggregate_charges(&models, day("2026-03-01")).unwrap();
    let order: Vec<(&str, &str)> = charges
        .iter()
        .map(|charge| (charge.team_id.as_str(), charge.model_id.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ("team-a", "model-a"),
            ("team-a", "model-z"),
            ("team-b", "model-a")
        ]
    );
}

#[test]
fn charges_outside_the_day_are_dropped() {
    let models = [
        ptu_model("model-a", "a", "team-a", 1, Decimal::ONE),
        windowed("2026-03-02T00:00:00Z", None),
    ];
    let charges = aggregate_charges(&models, day("2026-03-01")).unwrap();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].model_id, "model-a");
}

#[test]
fn repeated_identities_keep_one_charge_each_in_input_order() {
    let models = [
        ptu_model("model-a", "first", "team-a", 1, Decimal::ONE),
        ptu_model("model-a", "second", "team-a", 1, Decimal::ONE),
    ];
    let charges = aggregate_charges(&models, day("2026-03-01")).unwrap();
    assert_eq!(charges.len(), 2);
    assert_eq!(charges[0].model_name, "first");
    assert_eq!(charges[1].model_name, "second");
}

#[test]
fn no_models_produce_no_charges() {
    assert!(
        aggregate_charges(&[], day("2026-03-01"))
            .unwrap()
            .is_empty()
    );
}
