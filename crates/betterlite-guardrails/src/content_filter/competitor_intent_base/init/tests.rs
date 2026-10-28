#![cfg(test)]

use super::BaseCompetitorIntentChecker;
use super::tests::config;

#[test]
fn init() {
    let checker = BaseCompetitorIntentChecker::new(&config());
    assert!((checker.threshold_high - 0.70_f64).abs() < f64::EPSILON);
    assert!((checker.threshold_medium - 0.45_f64).abs() < f64::EPSILON);
    assert!((checker.threshold_low - 0.30_f64).abs() < f64::EPSILON);
}

#[test]
fn init_configuration() {
    let settings = serde_json::from_str(
        r#"{
        "brand_self": [" Our Brand ", ""], "competitors": [" ACME ", ""],
        "competitor_aliases": {"acme": [" AC ", "ACE", " "]}, "locations": ["ACME"],
        "policy": {"competitor_comparison": "refuse"},
        "threshold_high": 0.8, "threshold_medium": 0.6, "threshold_low": 0.4,
        "reframe_message_template": "reframe", "refuse_message_template": "refuse",
        "comparison_words": ["prefer"], "domain_words": [" AIRLINE "]
    }"#,
    )
    .unwrap();
    let checker = BaseCompetitorIntentChecker::new(&settings);
    assert_eq!(checker.brand_self, ["our brand"]);
    assert_eq!(checker.competitor_tokens.len(), 3);
    assert!(checker.ambiguous.contains("acme"));
    assert_eq!(
        checker
            .policy
            .get("competitor_comparison")
            .map(String::as_str),
        Some("refuse")
    );
    assert!((checker.threshold_high - 0.8_f64).abs() < f64::EPSILON);
    assert!((checker.threshold_medium - 0.6_f64).abs() < f64::EPSILON);
    assert!((checker.threshold_low - 0.4_f64).abs() < f64::EPSILON);
    assert_eq!(checker.reframe_message_template.as_deref(), Some("reframe"));
    assert_eq!(checker.refuse_message_template.as_deref(), Some("refuse"));
    assert_eq!(checker.comparison_words, ["prefer"]);
    assert_eq!(checker.domain_words, ["airline"]);
}

#[test]
fn init_empty_comparison_defaults() {
    let settings = serde_json::from_str(r#"{"comparison_words": []}"#).unwrap();
    let checker = BaseCompetitorIntentChecker::new(&settings);
    assert_eq!(checker.comparison_words.len(), 9);
}

#[test]
fn init_unknown_key() {
    let error =
        serde_json::from_str::<super::configuration::Settings>(r#"{"unknown": 1}"#).unwrap_err();
    assert!(error.to_string().contains("unknown"));
}

#[test]
fn init_blank_entries() {
    let settings = serde_json::from_str(
        r#"{
        "brand_self": ["", " "], "competitors": ["", " "],
        "locations": ["", " "], "domain_words": ["", " "]
    }"#,
    )
    .unwrap();
    let checker = BaseCompetitorIntentChecker::new(&settings);
    assert_eq!(checker.brand_self, [""]);
    assert_eq!(checker.domain_words, [""]);
    assert!(checker.ambiguous.contains(""));
}
