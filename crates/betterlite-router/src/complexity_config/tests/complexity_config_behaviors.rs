use std::collections::BTreeMap;

use betterlite_config::sections::complexity_config as section;
use betterlite_config::sections::custom_dimensions as dimensions_section;
use betterlite_config::sections::tier_definitions as tier_section;
use rstest::rstest;

use crate::complexity_config::checks::{Checks, Declared, Feature};
use crate::complexity_config::error::ComplexityConfigError;
use crate::complexity_config::resolve;
use crate::tier_definitions::checks::ClassifierKind;
use crate::tier_definitions::error::{DefinitionConflict, TierDefinitionsError};

use crate::complexity_config::tests::{
    base_checks, blank, checked, custom, error, llm, single, strings, tier_pools, tiers,
};

#[test]
fn escalation_keywords_are_stripped_and_blank_entries_dropped() {
    let settings = section::Config {
        escalation_keywords: Some(strings(&["  escalate  ", "   ", "\u{1c}hard\u{1f}"])),
        ..blank()
    };
    let resolved = checked(settings, tiers(), &base_checks());
    assert_eq!(
        resolved.escalation_keywords,
        Some(strings(&["escalate", "hard"]))
    );
}

#[test]
fn an_absent_keyword_list_stays_absent_and_an_empty_one_stays_empty() {
    let absent = checked(blank(), tiers(), &base_checks());
    assert_eq!(absent.escalation_keywords, None);
    let settings = section::Config {
        escalation_keywords: Some(Vec::new()),
        ..blank()
    };
    let empty = checked(settings, tiers(), &base_checks());
    assert_eq!(empty.escalation_keywords, Some(Vec::new()));
}

#[test]
fn plan_mode_and_housekeeping_patterns_are_stripped() {
    let settings = section::Config {
        plan_mode_patterns: Some(strings(&[" Plan mode is active ", "  "])),
        housekeeping_patterns: Some(strings(&["  Write the title  ", ""])),
        ..blank()
    };
    let resolved = checked(settings, tiers(), &base_checks());
    assert_eq!(
        resolved.plan_mode_patterns,
        Some(strings(&["Plan mode is active"]))
    );
    assert_eq!(
        resolved.housekeeping_patterns,
        Some(strings(&["Write the title"]))
    );
}

#[test]
fn a_padded_plan_mode_floor_is_trimmed() {
    let settings = section::Config {
        plan_mode_min_tier: Some("  COMPLEX  ".to_owned()),
        ..blank()
    };
    let resolved = checked(settings, tiers(), &base_checks());
    assert_eq!(resolved.plan_mode_min_tier.as_deref(), Some("COMPLEX"));
}

#[test]
fn a_custom_tier_set_accepts_a_plan_mode_floor_naming_a_defined_tier() {
    let settings = section::Config {
        plan_mode_min_tier: Some("ECO".to_owned()),
        ..blank()
    };
    let resolved = checked(settings, custom(), &llm("ECO"));
    assert_eq!(resolved.plan_mode_min_tier.as_deref(), Some("ECO"));
}

#[test]
fn a_padded_fallback_tier_is_trimmed_before_the_tier_set_reads_it() {
    let checks = Checks {
        fallback_tier: Some("  ECO  "),
        ..llm("ECO")
    };
    let resolved = checked(blank(), custom(), &checks);
    assert_eq!(resolved.tiers.fallback.as_deref(), Some("ECO"));
}

#[test]
fn a_blank_fallback_tier_is_rejected() {
    let checks = Checks {
        fallback_tier: Some("   "),
        ..llm("ECO")
    };
    let rejected = error(blank(), custom(), &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::BlankText {
            field: "fallback_tier"
        }
    ));
}

#[test]
fn classification_instructions_are_trimmed_and_kept_separate() {
    let checks = Checks {
        classifier_type: ClassifierKind::Llm,
        classification_prompt: Some("  pick a tier  "),
        classification_examples: Some("\n one example \n"),
        ..base_checks()
    };
    let resolved = checked(blank(), tiers(), &checks);
    assert_eq!(
        resolved.classification_prompt.as_deref(),
        Some("pick a tier")
    );
    assert_eq!(
        resolved.classification_examples.as_deref(),
        Some("one example")
    );
}

#[test]
fn keyword_rules_may_name_active_tiers() {
    let rules = strings(&["SIMPLE", "REASONING"]);
    let checks = Checks {
        keyword_rule_tiers: &rules,
        ..base_checks()
    };
    let resolved = checked(blank(), tiers(), &checks);
    assert_eq!(
        resolved.tiers.tier_names(),
        ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]
    );
}

#[test]
fn keyword_rules_reject_unknown_tiers() {
    let rules = strings(&["PREMIUM"]);
    let checks = Checks {
        keyword_rule_tiers: &rules,
        ..base_checks()
    };
    let rejected = error(blank(), tiers(), &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::KeywordRuleUnknownTiers { tiers, valid }
            if tiers == strings(&["PREMIUM"]) && valid == "SIMPLE, MEDIUM, COMPLEX, REASONING"
    ));
}

#[test]
fn adaptive_accepts_configured_pools() {
    let features = [Feature::Adaptive];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[("SIMPLE", single("mini"))])),
        ..tiers()
    };
    let resolved = checked(blank(), tier_set, &checks);
    assert_eq!(resolved.tiers.pools.len(), 1);
}

#[test]
fn adaptive_rejects_an_explicitly_empty_pool() {
    let features = [Feature::Adaptive];
    let checks = Checks {
        features: &features,
        ..base_checks()
    };
    let tier_set = tier_section::Config {
        tier_models: Some(tier_pools(&[(
            "SIMPLE",
            tier_section::TierPool::Pool(Vec::new()),
        )])),
        ..tiers()
    };
    let rejected = error(blank(), tier_set, &checks);
    assert!(matches!(
        rejected,
        ComplexityConfigError::AdaptiveWithoutPools
    ));
}

#[test]
fn semantic_matching_accepts_a_model_and_a_rule() {
    let features = [Feature::SemanticKeywordMatching];
    let rules = strings(&["COMPLEX"]);
    let checks = Checks {
        features: &features,
        keyword_rule_tiers: &rules,
        embedding_model: Some("text-embedding-3-small"),
        ..base_checks()
    };
    let resolved = checked(blank(), tiers(), &checks);
    assert_eq!(resolved.tiers.tier_names().len(), 4);
}

#[test]
fn heuristic_first_accepts_a_threshold_below_the_highest_tier() {
    let checks = Checks {
        classifier_type: ClassifierKind::HeuristicFirst,
        ..base_checks()
    };
    let settings = section::Config {
        heuristic_first_max_tier: Some("  MEDIUM  ".to_owned()),
        ..blank()
    };
    let resolved = checked(settings, tiers(), &checks);
    assert_eq!(resolved.heuristic_first_max_tier.as_deref(), Some("MEDIUM"));
}

#[test]
fn hybrid_accepts_a_boundary_margin() {
    let checks = Checks {
        classifier_type: ClassifierKind::Hybrid,
        ..base_checks()
    };
    let settings = section::Config {
        hybrid_boundary_margin: Some(0.5_f64),
        ..blank()
    };
    let resolved = checked(settings, tiers(), &checks);
    assert_eq!(resolved.hybrid_boundary_margin, Some(0.5_f64));
}

#[test]
fn a_blank_only_keyword_list_holds_no_order_feature() {
    let settings = section::Config {
        escalation_keywords: Some(strings(&["   ", "\u{1c}"])),
        ..blank()
    };
    let resolved = checked(settings, custom(), &llm("ECO"));
    assert_eq!(resolved.escalation_keywords, Some(Vec::new()));
    assert_eq!(resolved.tiers.tier_names(), ["ECO", "DEEP"]);
}

fn order_conflict(rejected: &ComplexityConfigError) -> Vec<&'static str> {
    let ComplexityConfigError::Tiers(TierDefinitionsError::Conflicts { conflicts }) = rejected
    else {
        panic!("a custom tier set must report the order-dependent conflict");
    };
    conflicts
        .iter()
        .find_map(|conflict| match conflict {
            DefinitionConflict::OrderFeatures(features) => Some(features.clone()),
            DefinitionConflict::SystemPrompt
            | DefinitionConflict::ClassificationRubric
            | DefinitionConflict::DefaultModelFallback
            | DefinitionConflict::TierLabels => None,
        })
        .unwrap_or_default()
}

#[test]
fn a_real_keyword_list_conflicts_with_a_custom_tier_set() {
    let settings = section::Config {
        escalation_keywords: Some(strings(&["escalate"])),
        ..blank()
    };
    let rejected = error(settings, custom(), &llm("ECO"));
    assert_eq!(order_conflict(&rejected), ["escalation_keywords"]);
}

#[rstest]
#[case::adaptive(Feature::Adaptive, "adaptive")]
#[case::plugins(Feature::Plugins, "plugins")]
#[case::session_affinity(Feature::SessionAffinity, "session_affinity")]
#[case::stall_escalation(Feature::StallEscalation, "stall_escalation_enabled")]
fn every_order_dependent_feature_conflicts_with_a_custom_tier_set(
    #[case] feature: Feature,
    #[case] label: &str,
) {
    let features = [feature];
    let checks = Checks {
        features: &features,
        ..llm("ECO")
    };
    let rejected = error(blank(), custom(), &checks);
    assert_eq!(order_conflict(&rejected), [label]);
}

#[test]
fn dimensions_are_validated_and_carried_through() {
    let dimensions = [dimensions_section::Config {
        name: "domain".to_owned(),
        weight: 0.5_f64,
        keywords: strings(&["kubernetes"]),
        patterns: Vec::new(),
        scoring_mode: "binary".to_owned(),
    }];
    let weights = BTreeMap::new();
    let checks = base_checks();
    let resolved = resolve(&Declared {
        settings: blank(),
        tiers: tiers(),
        dimensions: &dimensions,
        dimension_weights: &weights,
        checks,
    })
    .expect("the dimension resolves");
    assert_eq!(resolved.dimensions.dimensions.len(), 1);
}

#[test]
fn a_dimension_problem_surfaces_as_a_dimensions_error() {
    let dimensions = [dimensions_section::Config {
        name: String::new(),
        weight: 0.5_f64,
        keywords: strings(&["kubernetes"]),
        patterns: Vec::new(),
        scoring_mode: "binary".to_owned(),
    }];
    let weights = BTreeMap::new();
    let rejected = resolve(&Declared {
        settings: blank(),
        tiers: tiers(),
        dimensions: &dimensions,
        dimension_weights: &weights,
        checks: base_checks(),
    })
    .expect_err("an unnamed dimension is rejected");
    assert!(matches!(rejected, ComplexityConfigError::Dimensions(_)));
}

#[test]
fn a_tier_set_problem_surfaces_as_a_tiers_error() {
    let tier_set = tier_section::Config {
        tier_definitions: Some(vec![tier_section::TierDefinition {
            name: "ONLY".to_owned(),
            description: Some("one tier".to_owned()),
        }]),
        tier_models: Some(tier_pools(&[("ONLY", single("model"))])),
        ..tiers()
    };
    let rejected = error(blank(), tier_set, &llm("ONLY"));
    assert!(matches!(
        rejected,
        ComplexityConfigError::Tiers(TierDefinitionsError::TierCount { .. })
    ));
}
