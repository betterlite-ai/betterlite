#![cfg(test)]

use super::shared::{airline_checker, airline_settings};
use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use crate::content_filter::competitor_intent_base::Meaning;

#[test]
fn explicit_competitor_marker_returns_high_confidence() {
    assert_eq!(
        airline_checker().classify_ambiguous("Qatar Airways", "qatar"),
        (Meaning::Competitor, 0.85_f64)
    );
}

#[test]
fn explicit_other_meaning_marker_returns_high_confidence() {
    assert_eq!(
        airline_checker().classify_ambiguous("fly to qatar", "qatar"),
        (Meaning::OtherMeaning, 0.85_f64)
    );
}

#[test]
fn operational_signal_without_comparison_is_other_meaning() {
    assert_eq!(
        airline_checker().classify_ambiguous("qatar lounge", "qatar"),
        (Meaning::OtherMeaning, 0.85_f64)
    );
}

#[test]
fn operational_signal_with_comparison_falls_through_to_scoring() {
    let (meaning, confidence) =
        airline_checker().classify_ambiguous("qatar lounge is better", "qatar");
    assert_eq!(meaning, Meaning::Competitor);
    assert!(confidence >= 0.6_f64);
}

#[test]
fn custom_explicit_competitor_marker_overrides_default() {
    let mut settings = airline_settings();
    settings.explicit_competitor_marker = Some(r"\bskyline\b".to_owned());
    let overridden = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        overridden.classify_ambiguous("qatar skyline program", "qatar"),
        (Meaning::Competitor, 0.85_f64)
    );
    assert_eq!(
        airline_checker().classify_ambiguous("qatar skyline program", "qatar"),
        (Meaning::OtherMeaning, 0.5_f64)
    );
}
