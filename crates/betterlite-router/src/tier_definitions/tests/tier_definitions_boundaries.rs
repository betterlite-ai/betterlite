use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions as section;
use rstest::rstest;

use crate::complexity_scoring::tier::ComplexityTier;
use crate::tier_definitions::checks::{
    ClassifierFallback, ClassifierKind, OrderFeature, TierChecks,
};
use crate::tier_definitions::error::{DefinitionConflict, TierDefinitionsError};
use crate::tier_definitions::model::label_for;
use crate::tier_definitions::tests::{
    base_checks, blank, checked, custom, custom_with, error, labels, ladder, llm, order_features,
    pool, pools, single,
};

// a two-tier custom set whose definition name and pool key differ, so a test can spell a padded name
fn named_definition(name: &str, pool_key: &str, description: Option<&str>) -> section::Config {
    let definitions = vec![
        section::TierDefinition {
            name: name.to_owned(),
            description: description.map(str::to_owned),
        },
        section::TierDefinition {
            name: "SECOND".to_owned(),
            description: Some("asks".to_owned()),
        },
    ];
    let mapped = BTreeMap::from([
        (pool_key.to_owned(), single("first-model")),
        ("SECOND".to_owned(), single("second-model")),
    ]);
    section::Config {
        tier_definitions: Some(definitions),
        tier_models: Some(mapped),
        ..blank()
    }
}

#[rstest]
#[case::lower_bound(2, true)]
#[case::upper_bound(8, true)]
#[case::below_the_lower_bound(1, false)]
#[case::above_the_upper_bound(9, false)]
fn the_definition_count_bounds_are_two_and_eight(#[case] count: usize, #[case] accepted: bool) {
    let config = custom_with(count);
    let checks = llm("TIER_0");
    if accepted {
        let set = checked(&config, &checks);
        assert_eq!(set.definitions.as_deref().unwrap().len(), count);
        return;
    }
    assert!(matches!(
        error(&config, &checks),
        TierDefinitionsError::TierCount { count: reported } if reported == count
    ));
}

// the same value bare or wrapped in the spaces a definition strips
fn spaced(value: &str, padded: bool) -> String {
    if padded {
        format!("  {value}  ")
    } else {
        value.to_owned()
    }
}

#[rstest]
#[case::bare(false)]
#[case::padded(true)]
fn definition_fields_at_the_character_limit_are_accepted(#[case] padded: bool) {
    let name = "x".repeat(64);
    let description = "d".repeat(500);
    let config = named_definition(
        &spaced(&name, padded),
        &name,
        Some(&spaced(&description, padded)),
    );
    let set = checked(&config, &llm("SECOND"));
    let definition = &set.definitions.as_deref().unwrap()[0];
    assert_eq!(definition.name, name);
    assert_eq!(
        definition.description.as_deref(),
        Some(description.as_str())
    );
}

#[rstest]
#[case::plain("x".repeat(65))]
#[case::padded(format!("  {}  ", "x".repeat(65)))]
fn a_definition_name_over_the_character_limit_is_rejected(#[case] name: String) {
    let config = named_definition(&name, name.trim(), Some("asks"));
    assert!(matches!(
        error(&config, &llm("SECOND")),
        TierDefinitionsError::NameTooLong { max: 64, .. }
    ));
}

#[test]
fn a_description_over_the_character_limit_is_rejected() {
    let description = "d".repeat(501);
    let config = named_definition("ECO", "ECO", Some(&description));
    assert!(matches!(
        error(&config, &llm("SECOND")),
        TierDefinitionsError::DescriptionTooLong { max: 500, .. }
    ));
}

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
fn an_empty_definition_name_is_rejected(#[case] name: &str) {
    let config = named_definition(name, "ECO", Some("asks"));
    assert!(matches!(
        error(&config, &llm("SECOND")),
        TierDefinitionsError::EmptyName
    ));
}

#[rstest]
#[case::absent(None)]
#[case::blank(Some("   "))]
fn a_custom_tier_needs_a_description(#[case] description: Option<&str>) {
    let config = named_definition("ECO", "ECO", description);
    assert!(matches!(
        error(&config, &llm("SECOND")),
        TierDefinitionsError::MissingDescription { name } if name == "ECO"
    ));
}

#[test]
fn a_built_in_name_inherits_its_description_in_any_case() {
    let config = named_definition("simple", "simple", None);
    let set = checked(&config, &llm("SECOND"));
    let definitions = set.definitions.as_deref().unwrap();
    assert_eq!(definitions[0].name, "simple");
    assert_eq!(definitions[0].description.as_deref(), None);
}

#[rstest]
#[case::in_the_name("A\nB", Some("asks"))]
#[case::in_the_description("A B", Some("asks\nmore"))]
#[case::carriage_return("A B", Some("asks\rmore"))]
fn a_newline_in_a_definition_is_rejected(#[case] name: &str, #[case] description: Option<&str>) {
    let config = named_definition(name, "A B", description);
    assert!(matches!(
        error(&config, &llm("SECOND")),
        TierDefinitionsError::Newline { .. }
    ));
}

#[test]
fn a_repeated_definition_name_is_rejected_case_insensitively() {
    let config = section::Config {
        tier_definitions: ladder(&[
            ("ECO", Some("fresh asks")),
            ("eco", Some("fresh asks")),
            ("DEEP", Some("hard asks")),
        ]),
        tier_models: pools(&[
            ("ECO", single("eco-mini")),
            ("eco", single("eco-small")),
            ("DEEP", single("deep-a")),
        ]),
        ..blank()
    };
    assert!(matches!(
        error(&config, &llm("ECO")),
        TierDefinitionsError::DuplicateNames { names } if names == ["ECO", "eco"]
    ));
}

#[test]
fn a_custom_set_without_a_fallback_is_rejected() {
    let checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        ..base_checks()
    };
    assert!(matches!(
        error(&custom(), &checks),
        TierDefinitionsError::FallbackRequired
    ));
}

#[test]
fn a_fallback_outside_the_defined_names_is_rejected() {
    assert!(matches!(
        error(&custom(), &llm("PREMIUM")),
        TierDefinitionsError::FallbackUnknown { tier, defined }
            if tier == "PREMIUM" && defined == ["ECO", "DEEP"]
    ));
}

#[test]
fn a_fallback_without_a_custom_set_is_rejected() {
    let checks = TierChecks {
        fallback_tier: Some("ECO"),
        ..base_checks()
    };
    assert!(matches!(
        error(&blank(), &checks),
        TierDefinitionsError::FallbackWithoutTiers
    ));
}

#[test]
fn every_defined_tier_needs_a_pool() {
    let config = section::Config {
        tier_models: pools(&[("ECO", single("eco-mini"))]),
        ..custom()
    };
    assert!(matches!(
        error(&config, &llm("ECO")),
        TierDefinitionsError::MissingTiers { names } if names == ["DEEP"]
    ));
}

#[test]
fn a_pool_for_an_undefined_tier_is_rejected() {
    let config = section::Config {
        tier_models: pools(&[
            ("ECO", single("eco-mini")),
            ("DEEP", pool(&["deep-a"])),
            ("EXTRA", single("extra")),
        ]),
        ..custom()
    };
    assert!(matches!(
        error(&config, &llm("ECO")),
        TierDefinitionsError::UnknownTiers { names } if names == ["EXTRA"]
    ));
}

#[rstest]
#[case::single_empty_name(single(""), "ECO")]
#[case::empty_pool(pool(&[]), "ECO")]
fn an_empty_pool_is_rejected(#[case] configured: section::TierPool, #[case] tier: &str) {
    let config = section::Config {
        tier_models: pools(&[("ECO", configured), ("DEEP", pool(&["deep-a"]))]),
        ..custom()
    };
    assert!(matches!(
        error(&config, &llm("ECO")),
        TierDefinitionsError::EmptyPools { names } if names == [tier]
    ));
}

#[test]
fn empty_pools_are_reported_sorted() {
    let config = section::Config {
        tier_models: pools(&[("ECO", single("")), ("DEEP", pool(&[]))]),
        ..custom()
    };
    assert!(matches!(
        error(&config, &llm("ECO")),
        TierDefinitionsError::EmptyPools { names } if names == ["DEEP", "ECO"]
    ));
}

#[test]
fn a_non_reasoning_pool_without_the_flag_is_rejected() {
    let config = section::Config {
        tier_models: pools(&[("NON_REASONING", pool(&[]))]),
        ..blank()
    };
    assert!(matches!(
        error(&config, &TierChecks::default()),
        TierDefinitionsError::NonReasoningWithoutFlag { tier } if tier == "NON_REASONING"
    ));
}

#[rstest]
#[case::missing_pool(None)]
#[case::empty_pool(Some(pool(&[])))]
fn an_enabled_non_reasoning_tier_needs_a_pool(#[case] configured: Option<section::TierPool>) {
    let tier_models = configured.map(|pool| pools(&[("NON_REASONING", pool)]).unwrap());
    let config = section::Config {
        tier_models,
        non_reasoning_tier: true,
        ..blank()
    };
    let checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        ..base_checks()
    };
    assert!(matches!(
        error(&config, &checks),
        TierDefinitionsError::NonReasoningWithoutModel { tier } if tier == "NON_REASONING"
    ));
}

#[test]
fn an_enabled_non_reasoning_tier_needs_a_classifier_that_emits_it() {
    let config = section::Config {
        tier_models: pools(&[("NON_REASONING", single("mini"))]),
        non_reasoning_tier: true,
        ..blank()
    };
    assert!(matches!(
        error(&config, &TierChecks::default()),
        TierDefinitionsError::NonReasoningClassifier {
            classifier: ClassifierKind::Heuristic
        }
    ));
}

#[test]
fn an_enabled_non_reasoning_tier_cannot_join_a_custom_set() {
    let config = section::Config {
        non_reasoning_tier: true,
        ..custom()
    };
    assert!(matches!(
        error(&config, &llm("ECO")),
        TierDefinitionsError::NonReasoningWithTiers
    ));
}

#[test]
fn blank_labels_are_rejected_in_severity_order() {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "  "), ("MEDIUM", "")]),
        ..blank()
    };
    assert!(matches!(
        error(&config, &TierChecks::default()),
        TierDefinitionsError::BlankLabels { tiers }
            if tiers == [ComplexityTier::Simple, ComplexityTier::Medium]
    ));
}

#[test]
fn a_label_that_spells_another_tier_is_rejected() {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", " Reasoning ")]),
        ..blank()
    };
    assert!(matches!(
        error(&config, &TierChecks::default()),
        TierDefinitionsError::ShadowedLabels { labels }
            if labels == [(ComplexityTier::Simple, "Reasoning".to_owned())]
    ));
}

#[test]
fn a_label_that_spells_its_own_tier_is_accepted() {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "simple")]),
        ..blank()
    };
    let set = checked(&config, &TierChecks::default());
    assert_eq!(label_for(&set.labels, ComplexityTier::Simple), "simple");
}

#[test]
fn shared_labels_are_rejected_as_one_group() {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "Same"), ("MEDIUM", "same")]),
        ..blank()
    };
    assert!(matches!(
        error(&config, &TierChecks::default()),
        TierDefinitionsError::DuplicateLabels { groups }
            if groups == [vec![ComplexityTier::Simple, ComplexityTier::Medium]]
    ));
}

#[test]
fn a_label_for_an_unknown_tier_is_rejected() {
    let config = section::Config {
        tier_labels: labels(&[("ECO", "Cheap")]),
        ..blank()
    };
    assert!(matches!(
        error(&config, &TierChecks::default()),
        TierDefinitionsError::UnknownLabelTier { tier } if tier == "ECO"
    ));
}

#[rstest]
#[case::heuristic(ClassifierKind::Heuristic, false)]
#[case::heuristic_v2(ClassifierKind::HeuristicV2, false)]
#[case::capability(ClassifierKind::Capability, false)]
#[case::heuristic_first(ClassifierKind::HeuristicFirst, false)]
#[case::hybrid(ClassifierKind::Hybrid, false)]
#[case::llm(ClassifierKind::Llm, true)]
#[case::custom(ClassifierKind::Custom, true)]
#[case::jev(ClassifierKind::Jev, true)]
fn only_llm_custom_and_jev_classifiers_emit_custom_tiers(
    #[case] classifier: ClassifierKind,
    #[case] accepted: bool,
) {
    let checks = TierChecks {
        classifier_type: classifier,
        fallback_tier: Some("ECO"),
        ..base_checks()
    };
    if accepted {
        let set = checked(&custom(), &checks);
        assert_eq!(set.tier_names(), ["ECO", "DEEP"]);
        return;
    }
    assert!(matches!(
        error(&custom(), &checks),
        TierDefinitionsError::ClassifierCannotEmitTiers { classifier: reported }
            if reported == classifier
    ));
}

#[rstest]
#[case::llm(ClassifierKind::Llm, true, true)]
#[case::custom(ClassifierKind::Custom, false, true)]
#[case::jev(ClassifierKind::Jev, false, true)]
#[case::capability(ClassifierKind::Capability, true, false)]
#[case::llm_v2(ClassifierKind::LlmV2, true, false)]
#[case::heuristic_first(ClassifierKind::HeuristicFirst, true, false)]
#[case::hybrid(ClassifierKind::Hybrid, true, false)]
#[case::heuristic(ClassifierKind::Heuristic, false, false)]
#[case::heuristic_v2(ClassifierKind::HeuristicV2, false, false)]
fn the_llm_and_non_reasoning_classifier_sets_match_litellm(
    #[case] classifier: ClassifierKind,
    #[case] uses_llm: bool,
    #[case] emits_non_reasoning: bool,
) {
    assert_eq!(classifier.uses_llm(), uses_llm);
    let config = section::Config {
        tier_models: pools(&[("NON_REASONING", single("mini"))]),
        non_reasoning_tier: true,
        ..blank()
    };
    let checks = TierChecks {
        classifier_type: classifier,
        ..base_checks()
    };
    if emits_non_reasoning {
        let set = checked(&config, &checks);
        assert!(set.non_reasoning);
        return;
    }
    assert!(matches!(
        error(&config, &checks),
        TierDefinitionsError::NonReasoningClassifier { classifier: reported }
            if reported == classifier
    ));
}

#[test]
fn all_order_dependent_features_are_reported_in_one_conflict() {
    let features = [
        OrderFeature::Plugins,
        OrderFeature::Adaptive,
        OrderFeature::StallEscalation,
        OrderFeature::EscalationKeywords,
        OrderFeature::SessionAffinity,
    ];
    let checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        fallback_tier: Some("ECO"),
        classifier_fallback: ClassifierFallback::DefaultModel,
        llm_system_prompt: Some("whole prompt"),
        classification_rubric: Some("agentic"),
        order_features: &features,
        ..base_checks()
    };
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "Cheap")]),
        ..custom()
    };
    assert!(matches!(
        error(&config, &checks),
        TierDefinitionsError::Conflicts { conflicts }
            if conflicts == [
                DefinitionConflict::OrderFeatures(vec![
                    "adaptive",
                    "session_affinity",
                    "escalation_keywords",
                    "stall_escalation_enabled",
                    "plugins",
                ]),
                DefinitionConflict::SystemPrompt,
                DefinitionConflict::ClassificationRubric,
                DefinitionConflict::DefaultModelFallback,
                DefinitionConflict::TierLabels,
            ]
    ));
}

#[test]
fn a_classifier_conflict_names_the_configured_classifier() {
    let checks = TierChecks {
        classifier_type: ClassifierKind::Hybrid,
        fallback_tier: Some("ECO"),
        ..base_checks()
    };
    assert_eq!(
        error(&custom(), &checks).to_string(),
        "tier_definitions requires an LLM, Jev or custom classifier, got hybrid"
    );
}

#[test]
fn every_conflict_is_joined_into_one_message() {
    let features = [OrderFeature::Adaptive, OrderFeature::Plugins];
    let checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        fallback_tier: Some("ECO"),
        llm_system_prompt: Some("whole prompt"),
        order_features: &features,
        ..base_checks()
    };
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "Cheap")]),
        ..custom()
    };
    assert_eq!(
        error(&config, &checks).to_string(),
        "adaptive, plugins cannot be combined with tier_definitions: these features rely on the \
         built-in tier severity order; classifier_llm_config.system_prompt cannot be combined with \
         tier_definitions; tier_labels cannot be combined with tier_definitions"
    );
}

#[test]
fn a_default_model_fallback_is_allowed_without_custom_tiers() {
    let checks = TierChecks {
        classifier_fallback: ClassifierFallback::DefaultModel,
        ..base_checks()
    };
    let set = checked(&blank(), &checks);
    assert!(set.definitions.is_none());
}

#[test]
fn order_features_render_in_liteellm_order() {
    assert_eq!(
        order_features(&[OrderFeature::Plugins, OrderFeature::Adaptive]),
        ["adaptive", "plugins"]
    );
}
