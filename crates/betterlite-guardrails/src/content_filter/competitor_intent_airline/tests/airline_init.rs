#![cfg(test)]

use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use crate::content_filter::competitor_intent_airline::settings::AirlineSettings;

#[test]
fn default_domain_words_enable_category_ranking() {
    let checker = AirlineCompetitorIntentChecker::new(&AirlineSettings::default()).unwrap();
    assert_eq!(
        checker.run("best airline").intent.as_deref(),
        Some("category_ranking")
    );
}

#[test]
fn loads_major_airlines_when_competitors_not_configured() {
    let mut settings = AirlineSettings::default();
    settings.base.brand_self = vec!["Our Air".to_owned()];
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker.run("Emirates is better").intent.as_deref(),
        Some("competitor_comparison")
    );
}

#[test]
fn keeps_explicitly_configured_domain_words() {
    let mut settings = AirlineSettings::default();
    settings.base.domain_words = vec!["jet".to_owned()];
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker.run("best jet").intent.as_deref(),
        Some("category_ranking")
    );
    assert_eq!(checker.run("best airline").intent.as_deref(), Some("other"));
}
