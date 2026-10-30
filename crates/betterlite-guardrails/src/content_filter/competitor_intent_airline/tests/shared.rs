use super::*;
use crate::content_filter::competitor_intent_base::configuration::Settings;

fn settings() -> Settings {
    serde_json::from_str(
        r#"{"brand_self":["Our Air"],"competitors":["Qatar"],"locations":["Qatar"]}"#,
    )
    .unwrap()
}

#[test]
fn classify_ambiguous_base() {
    let checker = BaseCompetitorIntentChecker::new(&settings()).unwrap();
    let result = checker.run("qatar");
    assert_eq!(result.intent.as_deref(), Some("other"));
}

#[test]
fn find_matches_aliases() {
    let mut configured = settings();
    configured
        .competitor_aliases
        .insert("qatar".to_owned(), vec!["qr".to_owned()]);
    let checker = AirlineCompetitorIntentChecker::new(&configured).unwrap();
    let result = checker.run("Qatar and QR are better");
    assert_eq!(result.entities["competitors"], ["qatar"]);
}

#[test]
fn run_non_competitor() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(checker.run("hello").intent.as_deref(), Some("other"));
}

#[test]
fn run_competitor_comparison() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.run("Qatar Airways is better").confidence,
        Some(0.75_f64)
    );
}

#[test]
fn run_possible_competitor() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.run("Qatar airline is available").intent.as_deref(),
        Some("possible_competitor_comparison")
    );
}

#[test]
fn run_log_only() {
    let mut configured = settings();
    configured.threshold_high = 0.8_f64;
    configured.threshold_medium = 0.8_f64;
    let checker = AirlineCompetitorIntentChecker::new(&configured).unwrap();
    assert_eq!(
        checker.run("Qatar airline is available").intent.as_deref(),
        Some("log_only")
    );
}

#[test]
fn run_category_ranking() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(checker.run("best airline").confidence, Some(0.65_f64));
}

#[test]
fn confidence_with_comparison() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.run("Qatar Airways is better").confidence,
        Some(0.75_f64)
    );
}

#[test]
fn confidence_without_comparison() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.run("Qatar airline is available").confidence,
        Some(0.55_f64)
    );
}

#[test]
fn category_ranking_confidence() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(checker.run("best airline").confidence, Some(0.65_f64));
}

#[test]
fn ambiguous_token_context() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.run("flight to Qatar").intent.as_deref(),
        Some("other")
    );
    assert_eq!(
        checker.run("Qatar Airways").intent.as_deref(),
        Some("possible_competitor_comparison")
    );
}

#[test]
fn evidence_entries() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    let result = checker.run("Qatar Airways is better");
    assert!(
        result
            .evidence
            .iter()
            .any(|entry| entry.key.as_deref() == Some("competitor"))
    );
    assert!(
        result
            .evidence
            .iter()
            .any(|entry| entry.key.as_deref() == Some("comparison"))
    );
}

#[test]
fn action_hint_defaults() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(checker.run("hello").action_hint.as_deref(), Some("allow"));
    assert_eq!(
        checker.run("best airline").action_hint.as_deref(),
        Some("reframe")
    );
}

#[test]
fn load_competitors_brand() {
    let competitors = load_competitors_excl_brand(&["Qatar".to_owned()]);
    assert!(!competitors.iter().any(|value| value == "qatar"));
    assert!(competitors.iter().any(|value| value == "emirates"));
}

#[test]
fn airline_init_defaults() {
    let checker = AirlineCompetitorIntentChecker::new(&Settings::default()).unwrap();
    assert_eq!(
        checker.run("best airline").intent.as_deref(),
        Some("category_ranking")
    );
}

#[test]
fn airline_classify_markers() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.classify_ambiguous("Qatar Airways", "qatar"),
        (Meaning::Competitor, 0.85_f64)
    );
    assert_eq!(
        checker.classify_ambiguous("flight to qatar", "qatar"),
        (Meaning::OtherMeaning, 0.9_f64)
    );
}

#[test]
fn signal_scoring_ops() {
    let checker = AirlineCompetitorIntentChecker::new(&settings()).unwrap();
    assert_eq!(
        checker.classify_ambiguous("qatar lounge", "qatar"),
        (Meaning::OtherMeaning, 0.85_f64)
    );
    let (_, confidence) = checker.classify_ambiguous("qatar airline", "qatar");
    assert!(confidence >= 0.6_f64);
}
