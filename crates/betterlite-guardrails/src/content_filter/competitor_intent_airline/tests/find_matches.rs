#![cfg(test)]

use super::shared::airline_settings;
use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;

#[test]
fn alias_and_canonical_matches_resolve_to_one_competitor_entity() {
    let mut settings = airline_settings();
    settings
        .base
        .competitor_aliases
        .insert("qatar".to_owned(), vec!["qr".to_owned()]);
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    let result = checker.run("Qatar Airways and QR Airways are both great");
    assert_eq!(result.entities["competitors"], ["qatar"]);
}
