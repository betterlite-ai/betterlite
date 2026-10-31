#![cfg(test)]

use crate::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use crate::content_filter::competitor_intent_airline::settings::AirlineSettings;
use crate::content_filter::competitor_intent_base::configuration::Settings;

pub(super) fn base_config() -> Settings {
    serde_json::from_str(
        r#"{"brand_self":["Our Air"],"competitors":["Qatar"],"locations":["Qatar"]}"#,
    )
    .unwrap()
}

pub(super) fn airline_settings() -> AirlineSettings {
    AirlineSettings {
        base: base_config(),
        other_meaning_signals: Vec::new(),
        competitor_signals: Vec::new(),
        other_meaning_anchors: Vec::new(),
        explicit_competitor_marker: None,
        explicit_other_meaning_marker: None,
    }
}

pub(super) fn airline_checker() -> AirlineCompetitorIntentChecker {
    AirlineCompetitorIntentChecker::new(&airline_settings()).unwrap()
}
