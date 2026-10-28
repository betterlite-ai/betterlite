#![cfg(test)]

use super::BaseCompetitorIntentChecker;
use super::tests::{competitor, config};
use std::collections::BTreeMap;

#[test]
fn run_comparison() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run_with("Is AC better than our brand?", competitor);
    assert_eq!(result.intent.as_deref(), Some("competitor_comparison"));
    assert_eq!(result.confidence, Some(0.75_f64));
}

#[test]
fn run_category_ranking() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run("What is the best airline?");
    assert_eq!(result.intent.as_deref(), Some("category_ranking"));
    assert_eq!(result.confidence, Some(0.65_f64));
}

#[test]
fn run_literal_boundaries() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    assert_eq!(
        checker.run("best airlineish").intent.as_deref(),
        Some("other")
    );
}

#[test]
fn run_multiple_brands() {
    let mut configured = config();
    configured.brand_self.push("second brand".to_owned());
    let checker = BaseCompetitorIntentChecker::new(&configured).unwrap();
    assert_eq!(
        checker
            .run("our brand and second brand")
            .entities
            .get("brand_self"),
        Some(&vec!["our brand".to_owned(), "second brand".to_owned()])
    );
}

#[test]
fn run_safe_default() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run("acme and ac are better");
    assert_eq!(result.intent.as_deref(), Some("other"));
    assert_eq!(result.confidence, Some(0.0_f64));
    assert_eq!(result.action_hint.as_deref(), Some("allow"));
    assert_eq!(result.signals, ["other_meaning_or_ambiguous"]);
    assert_eq!(result.entities.get("competitors"), Some(&Vec::new()));
    assert_eq!(result.evidence.len(), 2);
    assert!(
        result
            .evidence
            .iter()
            .all(|entry| entry.key.as_deref() == Some("other_meaning") && entry.value.is_none())
    );
}

#[rstest::rstest]
#[case::high(0.75_f64, 0.6_f64, 0.3_f64, "competitor_comparison", "refuse")]
#[case::medium(
    0.8_f64,
    0.75_f64,
    0.3_f64,
    "possible_competitor_comparison",
    "reframe"
)]
#[case::low(0.8_f64, 0.8_f64, 0.75_f64, "log_only", "log_only")]
#[case::below(0.8_f64, 0.8_f64, 0.8_f64, "other", "allow")]
#[case::crossed(0.7_f64, 0.5_f64, 0.8_f64, "competitor_comparison", "refuse")]
fn run_thresholds(
    #[case] high: f64,
    #[case] medium: f64,
    #[case] low: f64,
    #[case] intent: &str,
    #[case] action: &str,
) {
    let mut settings = config();
    settings.threshold_high = high;
    settings.threshold_medium = medium;
    settings.threshold_low = low;
    settings.policy = BTreeMap::from([
        ("competitor_comparison".to_owned(), "refuse".to_owned()),
        (
            "possible_competitor_comparison".to_owned(),
            "reframe".to_owned(),
        ),
        ("log_only".to_owned(), "refuse".to_owned()),
        ("other".to_owned(), "refuse".to_owned()),
    ]);
    let checker = BaseCompetitorIntentChecker::new(&settings).unwrap();
    let result = checker.run_with("ac is better", competitor);
    assert_eq!(result.intent.as_deref(), Some(intent));
    assert_eq!(result.action_hint.as_deref(), Some(action));
    assert_eq!(result.confidence, Some(0.75_f64));
    assert_eq!(result.signals, ["competitor_resolved", "comparison"]);
}

#[rstest::rstest]
#[case::no_comparison("ac is fine")]
#[case::substring("ac is bestest")]
fn run_without_comparison(#[case] text: &str) {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run_with(text, competitor);
    assert_eq!(
        result.intent.as_deref(),
        Some("possible_competitor_comparison")
    );
    assert_eq!(result.confidence, Some(0.55_f64));
    assert_eq!(result.action_hint.as_deref(), Some("allow"));
    assert_eq!(result.signals, ["competitor_resolved"]);
}

#[rstest::rstest]
#[case::below(0.649_f64, "other", 2)]
#[case::boundary(0.65_f64, "possible_competitor_comparison", 1)]
fn run_disambiguation(
    #[case] confidence: f64,
    #[case] intent: &str,
    #[case] evidence_count: usize,
) {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run_with("ACME", |text, token| {
        assert_eq!(text, "acme");
        assert_eq!(token, "acme");
        (super::Meaning::Competitor, confidence)
    });
    assert_eq!(result.intent.as_deref(), Some(intent));
    assert_eq!(result.evidence.len(), evidence_count);
    let entity = result.evidence.first().unwrap();
    assert_eq!(entity.r#type.as_deref(), Some("entity"));
    assert_eq!(entity.key.as_deref(), Some("competitor"));
    assert_eq!(entity.r#match.as_deref(), Some("acme"));
    assert_eq!(entity.value.as_deref(), Some("acme"));
}

#[test]
fn run_low_confidence_evidence() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run_with("acme", |_text, _token| {
        (super::Meaning::Competitor, 0.45_f64)
    });
    let evidence = result.evidence.last().unwrap();
    assert_eq!(evidence.key.as_deref(), Some("other_meaning_default"));
    assert_eq!(evidence.r#match.as_deref(), Some("confidence 0.45"));
    assert_eq!(evidence.r#type.as_deref(), Some("signal"));
    assert_eq!(evidence.value, None);
    assert_eq!(result.entities.get("competitors"), Some(&Vec::new()));
}

#[test]
fn run_custom_words_and_policy() {
    let mut settings = config();
    settings.comparison_words = vec!["prefer".to_owned()];
    settings
        .policy
        .insert("category_ranking".to_owned(), "refuse".to_owned());
    let checker = BaseCompetitorIntentChecker::new(&settings).unwrap();
    let result = checker.run("prefer an airline");
    assert_eq!(result.action_hint.as_deref(), Some("refuse"));
    assert_eq!(
        result.evidence.first().unwrap().r#match.as_deref(),
        Some("comparison + domain")
    );
    assert_eq!(checker.run("best airline").intent.as_deref(), Some("other"));
}

#[test]
fn run_deduplicated_entities() {
    let checker = BaseCompetitorIntentChecker::new(&config()).unwrap();
    let result = checker.run_with("our brand vs ac and acme", competitor);
    assert_eq!(
        result.entities.get("competitors"),
        Some(&vec!["acme".to_owned()])
    );
    assert_eq!(
        result.entities.get("brand_self"),
        Some(&vec!["our brand".to_owned()])
    );
    assert_eq!(result.evidence.len(), 4);
    let brand = result.evidence.first().unwrap();
    assert_eq!(brand.key.as_deref(), Some("brand_self"));
    assert_eq!(brand.r#match.as_deref(), Some("our brand"));
    assert_eq!(brand.value.as_deref(), Some("our brand"));
    let signal = result.evidence.last().unwrap();
    assert_eq!(signal.key.as_deref(), Some("comparison"));
    assert_eq!(signal.r#match.as_deref(), Some("comparison language"));
    assert_eq!(result.entities.get("category"), Some(&Vec::new()));
}
