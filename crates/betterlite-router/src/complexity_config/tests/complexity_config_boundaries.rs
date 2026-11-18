use std::collections::BTreeMap;

use betterlite_config::sections::complexity_config as section;
use betterlite_config::sections::tier_definitions as tier_section;
use betterlite_config::sections::tier_definitions::TierPool;
use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::complexity_config::checks::{Checks, Feature, StallWindow};
use crate::complexity_config::error::ComplexityConfigError;
use crate::complexity_config::normalize::{MAX_EXAMPLES_CHARS, MAX_PROMPT_CHARS};
use crate::tier_definitions::checks::ClassifierKind;

use crate::complexity_config::tests::{
    base_checks, blank, checked, custom, entry, error, llm, resolve_with, single, strings,
    tier_pools, tiers,
};

fn classifier(kind: ClassifierKind) -> Checks<'static> {
    Checks {
        classifier_type: kind,
        ..base_checks()
    }
}

fn with_margin(margin: f64) -> Result<(), ComplexityConfigError> {
    let settings = section::Config {
        hybrid_boundary_margin: Some(margin),
        ..blank()
    };
    resolve_with(settings, tiers(), &classifier(ClassifierKind::Hybrid)).map(|_resolved| ())
}

#[rstest]
#[case::zero(0.0_f64)]
#[case::one(1.0_f64)]
#[case::half(0.5_f64)]
fn a_margin_at_an_endpoint_is_accepted(#[case] margin: f64) {
    assert!(with_margin(margin).is_ok());
}

#[rstest]
#[case::below_zero(-0.000_001_f64)]
#[case::above_one(1.000_001_f64)]
#[case::not_a_number(f64::NAN)]
fn a_margin_outside_the_endpoints_is_rejected(#[case] margin: f64) {
    let Err(ComplexityConfigError::HybridMarginOutOfRange { value }) = with_margin(margin) else {
        panic!("the margin must be refused");
    };
    assert_eq!(value.to_bits(), margin.to_bits());
}

#[test]
fn the_prompt_cap_counts_characters() {
    let at_cap = "\u{e9}".repeat(MAX_PROMPT_CHARS);
    let accepted = Checks {
        classifier_type: ClassifierKind::Llm,
        classification_prompt: Some(&at_cap),
        ..base_checks()
    };
    checked(blank(), tiers(), &accepted);

    let over_cap = "\u{e9}".repeat(MAX_PROMPT_CHARS + 1);
    let rejected_checks = Checks {
        classifier_type: ClassifierKind::Llm,
        classification_prompt: Some(&over_cap),
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &rejected_checks);
    let ComplexityConfigError::TextTooLong { field, max } = rejected else {
        panic!("the long prompt must be refused");
    };
    assert_eq!(field, "classification_prompt");
    assert_eq!(max, MAX_PROMPT_CHARS);
}

#[test]
fn the_examples_cap_counts_characters() {
    let at_cap = "\u{e9}".repeat(MAX_EXAMPLES_CHARS);
    let accepted = Checks {
        classifier_type: ClassifierKind::Llm,
        classification_examples: Some(&at_cap),
        ..base_checks()
    };
    checked(blank(), tiers(), &accepted);

    let over_cap = "\u{e9}".repeat(MAX_EXAMPLES_CHARS + 1);
    let rejected_checks = Checks {
        classifier_type: ClassifierKind::Llm,
        classification_examples: Some(&over_cap),
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &rejected_checks);
    let ComplexityConfigError::TextTooLong { field, max } = rejected else {
        panic!("the long examples must be refused");
    };
    assert_eq!(field, "classification_examples");
    assert_eq!(max, MAX_EXAMPLES_CHARS);
}

#[rstest]
#[case::blank_prompt("classification_prompt")]
#[case::blank_examples("classification_examples")]
fn a_blank_operator_section_is_rejected(#[case] field: &'static str) {
    let checks = match field {
        "classification_prompt" => Checks {
            classification_prompt: Some(" \u{1c} "),
            ..classifier(ClassifierKind::Llm)
        },
        _ => Checks {
            classification_examples: Some("\n\t "),
            ..classifier(ClassifierKind::Llm)
        },
    };
    let rejected = error(blank(), tiers(), &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::BlankText { field: named } if named == field
    ));
}

#[rstest]
#[case::lowest("SIMPLE")]
#[case::middle("MEDIUM")]
#[case::below_highest("COMPLEX")]
fn a_heuristic_first_threshold_below_the_highest_tier_is_accepted(#[case] tier: &str) {
    let settings = section::Config {
        heuristic_first_max_tier: Some(tier.to_owned()),
        ..blank()
    };
    let resolved = checked(
        settings,
        tiers(),
        &classifier(ClassifierKind::HeuristicFirst),
    );
    assert_eq!(resolved.heuristic_first_max_tier.as_deref(), Some(tier));
}

#[test]
fn a_heuristic_first_threshold_at_the_highest_tier_is_rejected() {
    let settings = section::Config {
        heuristic_first_max_tier: Some("REASONING".to_owned()),
        ..blank()
    };
    let rejected = error(
        settings,
        tiers(),
        &classifier(ClassifierKind::HeuristicFirst),
    );
    let ComplexityConfigError::HeuristicFirstHighest { tier } = rejected else {
        panic!("the highest tier must be refused");
    };
    assert_eq!(tier, "REASONING");
}

#[test]
fn a_heuristic_first_threshold_that_is_blank_names_no_tier() {
    let settings = section::Config {
        heuristic_first_max_tier: Some("   ".to_owned()),
        ..blank()
    };
    let rejected = error(
        settings,
        tiers(),
        &classifier(ClassifierKind::HeuristicFirst),
    );
    let ComplexityConfigError::HeuristicFirstUnknownTier { tier, .. } = rejected else {
        panic!("a blank threshold must be refused");
    };
    assert_eq!(tier, "");
}

#[test]
fn heuristic_first_without_a_threshold_is_rejected() {
    let rejected = error(
        blank(),
        tiers(),
        &classifier(ClassifierKind::HeuristicFirst),
    );
    assert!(matches!(
        rejected,
        ComplexityConfigError::HeuristicFirstRequired
    ));
}

#[test]
fn a_threshold_with_another_classifier_is_rejected() {
    let settings = section::Config {
        heuristic_first_max_tier: Some("COMPLEX".to_owned()),
        ..blank()
    };
    let rejected = error(settings, tiers(), &classifier(ClassifierKind::Llm));
    let ComplexityConfigError::HeuristicFirstUnused { classifier } = rejected else {
        panic!("an unused threshold must be refused");
    };
    assert_eq!(classifier, ClassifierKind::Llm);
}

#[test]
fn a_heuristic_first_threshold_must_name_a_configured_tier() {
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[("SIMPLE", single("mini"))])),
        ..tiers()
    };
    let settings = section::Config {
        heuristic_first_max_tier: Some("MEDIUM".to_owned()),
        ..blank()
    };
    let rejected = error(
        settings,
        tier_set,
        &classifier(ClassifierKind::HeuristicFirst),
    );
    let ComplexityConfigError::HeuristicFirstWithoutModel { tier } = rejected else {
        panic!("an unconfigured threshold must be refused");
    };
    assert_eq!(tier, "MEDIUM");
}

#[test]
fn a_margin_with_another_classifier_is_rejected() {
    let settings = section::Config {
        hybrid_boundary_margin: Some(0.5_f64),
        ..blank()
    };
    let rejected = error(settings, tiers(), &classifier(ClassifierKind::Llm));
    let ComplexityConfigError::HybridMarginUnused { classifier } = rejected else {
        panic!("an unused margin must be refused");
    };
    assert_eq!(classifier, ClassifierKind::Llm);
}

#[test]
fn hybrid_without_a_margin_is_rejected() {
    let rejected = error(blank(), tiers(), &classifier(ClassifierKind::Hybrid));
    assert!(matches!(
        rejected,
        ComplexityConfigError::HybridMarginRequired
    ));
}

#[rstest]
#[case::active("COMPLEX", true)]
#[case::unknown("PREMIUM", false)]
fn a_plan_mode_floor_must_name_an_active_tier(#[case] tier: &str, #[case] accepted: bool) {
    let settings = section::Config {
        plan_mode_min_tier: Some(tier.to_owned()),
        ..blank()
    };
    let outcome = resolve_with(settings, tiers(), &base_checks());
    match (outcome, accepted) {
        (Ok(_), true) => {}
        (Err(ComplexityConfigError::PlanModeUnknownTier { tier: named, names }), false) => {
            assert_eq!(named, tier);
            assert_eq!(names, "SIMPLE, MEDIUM, COMPLEX, REASONING");
        }
        (other, _) => panic!("unexpected outcome: {other:?}"),
    }
}

#[test]
fn a_plan_mode_floor_must_name_a_configured_tier() {
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[("SIMPLE", single("mini"))])),
        ..tiers()
    };
    let settings = section::Config {
        plan_mode_min_tier: Some("MEDIUM".to_owned()),
        ..blank()
    };
    let rejected = error(settings, tier_set, &base_checks());
    let ComplexityConfigError::PlanModeWithoutModel { tier } = rejected else {
        panic!("an unconfigured floor must be refused");
    };
    assert_eq!(tier, "MEDIUM");
}

#[test]
fn a_custom_tier_set_reports_the_defined_names_to_the_floor() {
    let settings = section::Config {
        plan_mode_min_tier: Some("PREMIUM".to_owned()),
        ..blank()
    };
    let rejected = error(settings, custom(), &llm("ECO"));
    let ComplexityConfigError::PlanModeUnknownTier { tier, names } = rejected else {
        panic!("the floor must be refused");
    };
    assert_eq!(tier, "PREMIUM");
    assert_eq!(names, "ECO, DEEP");
}

#[test]
fn unknown_keyword_rule_tiers_are_deduplicated_and_sorted() {
    let rules = strings(&["ZULU", "ALPHA", "ALPHA"]);
    let checks = Checks {
        keyword_rule_tiers: &rules,
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &checks);
    let ComplexityConfigError::KeywordRuleUnknownTiers { tiers, valid } = rejected else {
        panic!("unknown rule tiers must be refused");
    };
    assert_eq!(tiers, ["ALPHA", "ZULU"]);
    assert_eq!(valid, "SIMPLE, MEDIUM, COMPLEX, REASONING");
}

fn adaptive_pools(pools: &[(&str, tier_section::TierPool)]) -> Result<(), ComplexityConfigError> {
    let features = [Feature::Adaptive];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(pools)),
        ..tiers()
    };
    resolve_with(blank(), tier_set, &checks).map(|_resolved| ())
}

#[test]
fn one_empty_pool_among_configured_ones_is_named() {
    let rejected = adaptive_pools(&[
        ("SIMPLE", TierPool::Pool(Vec::new())),
        ("MEDIUM", single("mini")),
    ])
    .expect_err("an empty pool is refused");
    let ComplexityConfigError::AdaptiveEmptyPools { tiers } = rejected else {
        panic!("the empty pool must be named");
    };
    assert_eq!(tiers, ["SIMPLE"]);
}

#[test]
fn every_pool_empty_is_the_no_pool_error() {
    let rejected = adaptive_pools(&[("SIMPLE", TierPool::Pool(Vec::new()))])
        .expect_err("no configured pool is refused");
    assert!(matches!(
        rejected,
        ComplexityConfigError::AdaptiveWithoutPools
    ));
    let empty_map = tier_section::Config {
        tier_models: Some(BTreeMap::new()),
        ..tiers()
    };
    let features = [Feature::Adaptive];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let rejected =
        resolve_with(blank(), empty_map, &checks).expect_err("an empty tier map is refused");
    assert!(matches!(
        rejected,
        ComplexityConfigError::AdaptiveWithoutPools
    ));
}

#[test]
fn an_empty_single_model_is_not_an_empty_pool() {
    adaptive_pools(&[("SIMPLE", single(""))]).expect("a single model is a one-element pool");
    adaptive_pools(&[(
        "SIMPLE",
        TierPool::Pool(vec![tier_section::TierEntry::Model(String::new())]),
    )])
    .expect("a one-entry pool is not empty");
}

#[test]
fn adaptive_needs_every_pool_configured() {
    adaptive_pools(&[("SIMPLE", single("mini")), ("MEDIUM", pool_two())])
        .expect("both pools are configured");
}

fn pool_two() -> tier_section::TierPool {
    TierPool::Pool(vec![
        tier_section::TierEntry::Model("a".to_owned()),
        tier_section::TierEntry::Model("b".to_owned()),
    ])
}

#[test]
fn semantic_matching_needs_an_embedding_model() {
    let features = [Feature::SemanticKeywordMatching];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::SemanticWithoutEmbeddingModel
    ));
}

#[test]
fn semantic_matching_needs_a_keyword_rule() {
    let features = [Feature::SemanticKeywordMatching];
    let checks = Checks {
        features: &features,
        embedding_model: Some("text-embedding-3-small"),
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::SemanticWithoutKeywordRules
    ));
}

#[test]
fn an_explicitly_empty_embedding_model_is_a_set_value() {
    let features = [Feature::SemanticKeywordMatching];
    let rules = strings(&["SIMPLE"]);
    let checks = Checks {
        features: &features,
        embedding_model: Some(""),
        keyword_rule_tiers: &rules,
        ..base_checks()
    };
    checked(blank(), tiers(), &checks);
}

#[test]
fn plugins_with_adaptive_are_rejected() {
    let features = [Feature::Plugins, Feature::Adaptive];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[("SIMPLE", single("mini"))])),
        ..tiers()
    };
    let rejected = error(blank(), tier_set, &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::PluginsWithAdaptive
    ));
}

#[test]
fn plugins_alone_are_accepted() {
    let features = [Feature::Plugins];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    checked(blank(), tiers(), &checks);
}

#[rstest]
#[case::session_affinity(Feature::SessionAffinity)]
#[case::user_turn(Feature::UserTurnClassification)]
fn stall_escalation_conflicts_with_a_held_decision(#[case] held: Feature) {
    let features = [Feature::StallEscalation, held];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::StallWithHeldDecision
    ));
}

#[rstest]
#[case::at_the_window(3, 3, true)]
#[case::defaults(6, 3, true)]
#[case::above_the_window(3, 4, false)]
fn a_stall_threshold_must_fit_the_window(
    #[case] window: u32,
    #[case] threshold: u32,
    #[case] accepted: bool,
) {
    let features = [Feature::StallEscalation];
    let checks = Checks {
        features: &features,
        stall_escalation: StallWindow {
            window,
            repeat_threshold: threshold,
        },
        ..base_checks()
    };
    let outcome = resolve_with(blank(), tiers(), &checks);
    match (outcome, accepted) {
        (Ok(_), true) => {}
        (
            Err(ComplexityConfigError::StallThresholdAboveWindow {
                threshold: reported,
                window: reported_window,
            }),
            false,
        ) => {
            assert_eq!(reported, threshold);
            assert_eq!(reported_window, window);
        }
        (other, _) => panic!("unexpected outcome: {other:?}"),
    }
}

#[test]
fn a_router_key_in_a_tier_entry_is_reported_with_its_tier() {
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[(
            "SIMPLE",
            TierPool::Single(entry(
                "mini",
                &[
                    ("fallback_tier", Json::Text("SIMPLE".to_owned())),
                    (
                        "heuristic_v2_artifact",
                        Json::Text("ultrafeedback".to_owned()),
                    ),
                ],
            )),
        )])),
        ..tiers()
    };
    let rejected = error(blank(), tier_set, &base_checks());
    let ComplexityConfigError::MisplacedTierParams { placements } = rejected else {
        panic!("the misplaced key must be reported");
    };
    assert_eq!(
        placements,
        ["SIMPLE.fallback_tier", "SIMPLE.heuristic_v2_artifact"]
    );
}

#[test]
fn every_entry_of_a_pool_reports_its_own_misplaced_keys() {
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[(
            "SIMPLE",
            TierPool::Pool(vec![
                entry("first", &[("adaptive", Json::Text("true".to_owned()))]),
                entry("second", &[("adaptive", Json::Text("true".to_owned()))]),
            ]),
        )])),
        ..tiers()
    };
    let rejected = error(blank(), tier_set, &base_checks());
    let ComplexityConfigError::MisplacedTierParams { placements } = rejected else {
        panic!("the misplaced key must be reported");
    };
    assert_eq!(placements, ["SIMPLE.adaptive", "SIMPLE.adaptive"]);
}

#[test]
fn a_request_parameter_that_is_not_a_router_key_is_left_alone() {
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[(
            "SIMPLE",
            TierPool::Single(entry(
                "mini",
                &[("temperature", Json::Text("0.2".to_owned()))],
            )),
        )])),
        ..tiers()
    };
    checked(blank(), tier_set, &base_checks());
}
