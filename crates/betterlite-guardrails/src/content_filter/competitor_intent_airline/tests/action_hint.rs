#![cfg(test)]

use super::shared::airline_settings;
use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;

#[test]
fn defaults_to_allow_for_non_competitor() {
    let settings = airline_settings();
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(checker.run("hello").action_hint.as_deref(), Some("allow"));
}

#[test]
fn defaults_to_reframe_for_category_ranking() {
    let settings = airline_settings();
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker.run("best airline").action_hint.as_deref(),
        Some("reframe")
    );
}

#[test]
fn defaults_to_log_only_for_log_only_intent() {
    let mut settings = airline_settings();
    settings.base.threshold_high = 0.8_f64;
    settings.base.threshold_medium = 0.8_f64;
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker
            .run("Qatar airline is available")
            .action_hint
            .as_deref(),
        Some("log_only")
    );
}

#[test]
fn policy_overrides_action_hint() {
    let mut settings = airline_settings();
    settings
        .base
        .policy
        .insert("competitor_comparison".to_owned(), "refuse".to_owned());
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker
            .run("Qatar Airways is better")
            .action_hint
            .as_deref(),
        Some("refuse")
    );
}
