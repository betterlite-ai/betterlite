#![cfg(test)]

use super::shared::airline_settings;
use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;

#[test]
fn low_confidence_below_medium_threshold_is_log_only() {
    let mut settings = airline_settings();
    settings.base.threshold_high = 0.8_f64;
    settings.base.threshold_medium = 0.8_f64;
    let checker = AirlineCompetitorIntentChecker::new(&settings).unwrap();
    assert_eq!(
        checker.run("Qatar airline is available").intent.as_deref(),
        Some("log_only")
    );
}
