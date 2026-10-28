#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentConfig;
use rstest::rstest;

use super::{
    BaseCompetitorIntentChecker, compile_marker as compile_marker_impl,
    count_signals as count_signals_impl, normalize as normalize_impl,
    text_for_entity_matching as text_for_entity_matching_impl,
    word_boundary_match as word_boundary_match_impl,
};

fn config() -> CompetitorIntentConfig {
    CompetitorIntentConfig {
        competitor_intent_type: None,
        brand_self: vec!["Our Brand".to_owned()],
        competitors: vec!["Acme".to_owned()],
        competitor_aliases: BTreeMap::from([(String::from("Acme"), String::from("AC"))]),
        domain_words: vec!["airline".to_owned()],
        route_geo_cues: vec!["acme".to_owned()],
        descriptor_lexicon: Vec::new(),
        indirect_competitor_patterns: BTreeMap::new(),
        policy: BTreeMap::new(),
        threshold_high: None,
        threshold_medium: None,
        threshold_low: None,
        reframe_message_template: None,
        refuse_message_template: None,
    }
}

#[rstest]
#[case::normalization("  B4t\u{200b}er  Ｃafe  ", "bater cafe")]
#[case::leet("@cme  4irline", "acme airline")]
fn normalize(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(normalize_impl(input), expected);
}

#[test]
fn text_for_entity_matching() {
    assert_eq!(text_for_entity_matching_impl("Acme, Inc.!"), "Acme Inc");
}

#[test]
fn word_boundary() {
    assert!(word_boundary_match_impl("compare acme today", "acme"));
    assert!(!word_boundary_match_impl("acmeish today", "acme"));
}

#[test]
fn count_signals() {
    let patterns = vec![
        "better".to_owned(),
        "(? invalid".to_owned(),
        "compare".to_owned(),
    ];
    assert_eq!(
        count_signals_impl("Is this better to compare?", &patterns),
        2
    );
}

#[test]
fn compile_marker() {
    assert!(compile_marker_impl(None).is_none());
    assert!(compile_marker_impl(Some(" ")).is_none());
    assert!(compile_marker_impl(Some("[")).is_none());
    assert!(compile_marker_impl(Some("compare")).is_some());
}

#[test]
fn classify_ambiguous() {
    let checker = BaseCompetitorIntentChecker::new(&config());
    assert_eq!(
        checker.classify_ambiguous("acme", "acme"),
        ("OTHER_MEANING".to_owned(), 0.5_f64)
    );
}

#[test]
fn init() {
    let checker = BaseCompetitorIntentChecker::new(&config());
    assert_eq!(checker.threshold_high, 0.70_f64);
    assert_eq!(checker.threshold_medium, 0.45_f64);
    assert_eq!(checker.threshold_low, 0.30_f64);
}

#[test]
fn find_matches() {
    let checker = BaseCompetitorIntentChecker::new(&config());
    let matches = checker.find_matches("Acme and AC");
    assert_eq!(matches.len(), 2);
    assert!(matches.iter().all(|(_, canonical, _)| canonical == "acme"));
}

#[test]
fn run_comparison() {
    let checker = BaseCompetitorIntentChecker::new(&config());
    let result = checker.run("Is AC better than our brand?");
    assert_eq!(result.intent.as_deref(), Some("competitor_comparison"));
    assert_eq!(result.confidence, Some(0.75_f64));
}

#[test]
fn run_category_ranking() {
    let checker = BaseCompetitorIntentChecker::new(&config());
    let result = checker.run("What is the best airline?");
    assert_eq!(result.intent.as_deref(), Some("category_ranking"));
    assert_eq!(result.confidence, Some(0.65_f64));
}
