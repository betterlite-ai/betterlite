#![cfg(test)]

use super::shared::airline_settings;
use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use crate::content_filter::competitor_intent_base::Meaning;

#[test]
fn zero_signals_default_to_other_meaning() {
    let checker = AirlineCompetitorIntentChecker::new(&airline_settings()).unwrap();
    assert_eq!(
        checker.classify_ambiguous("qatar", "qatar"),
        (Meaning::OtherMeaning, 0.5_f64)
    );
}

#[rstest::rstest]
#[case::airline_context("qatar has great miles and loyalty status", Meaning::Competitor)]
#[case::travel_context("flight to qatar via doha", Meaning::OtherMeaning)]
fn ratio_above_threshold_resolves_by_dominant_side(#[case] text: &str, #[case] expected: Meaning) {
    let checker = AirlineCompetitorIntentChecker::new(&airline_settings()).unwrap();
    let (meaning, confidence) = checker.classify_ambiguous(text, "qatar");
    assert_eq!(meaning, expected);
    assert!(confidence >= 0.6_f64);
    assert!(confidence <= 0.9_f64);
}

#[test]
fn custom_competitor_and_other_meaning_signals_are_used() {
    let mut settings = airline_settings();
    settings.competitor_signals = vec![r"\bmiles\b".to_owned()];
    settings.other_meaning_signals = vec![r"\bzzz\b".to_owned()];
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker
            .classify_ambiguous("qatar miles downtown office", "qatar")
            .0,
        Meaning::Competitor
    );
}

#[test]
fn custom_competitor_signal_supports_lookahead() {
    let mut settings = airline_settings();
    settings.competitor_signals = vec![r"\bmiles\b(?= balance)".to_owned()];
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();

    assert_eq!(
        checker.classify_ambiguous("qatar miles balance", "qatar"),
        (Meaning::Competitor, 0.9_f64)
    );
}

#[test]
fn custom_other_meaning_anchors_pull_the_ratio_toward_other_meaning() {
    let mut settings = airline_settings();
    settings.competitor_signals = vec![r"\bmiles\b".to_owned()];
    settings.other_meaning_signals = vec![r"\bzzz\b".to_owned()];
    settings.other_meaning_anchors = vec![r"\bdowntown\b".to_owned()];
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker
            .classify_ambiguous("qatar miles downtown office", "qatar")
            .0,
        Meaning::OtherMeaning
    );
}
