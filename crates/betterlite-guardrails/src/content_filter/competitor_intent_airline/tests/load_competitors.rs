#![cfg(test)]

use crate::content_filter::competitor_intent_airline::settings::AirlineSettings;
use crate::content_filter::competitor_intent_airline::{
    AirlineCompetitorIntentChecker, load_competitors_excl_brand,
};

#[test]
fn excludes_brand_self_match_variant_and_word() {
    let competitors = load_competitors_excl_brand(&["Qatar".to_owned()]);
    assert!(!competitors.iter().any(|value| value == "qatar"));
    assert!(!competitors.iter().any(|value| value == "qatar airways"));
    assert!(competitors.iter().any(|value| value == "emirates"));
}

#[test]
fn preserves_digit_only_airline_codes_without_leetspeak_translation() {
    let competitors = load_competitors_excl_brand(&[]);
    assert!(competitors.iter().any(|value| value == "5o"));
    assert!(!competitors.iter().any(|value| value == "so"));
}

#[test]
fn numeric_airline_code_does_not_hijack_an_ordinary_word() {
    let checker = AirlineCompetitorIntentChecker::new(&AirlineSettings::default()).unwrap();
    let result = checker.run("so which airline is best");
    assert_eq!(result.intent.as_deref(), Some("category_ranking"));
    assert_eq!(result.action_hint.as_deref(), Some("reframe"));
}
