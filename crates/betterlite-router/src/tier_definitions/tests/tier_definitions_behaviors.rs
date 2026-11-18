use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions as section;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::tier_definitions::checks::{ClassifierKind, TierChecks};
use crate::tier_definitions::error::TierDefinitionsError;
use crate::tier_definitions::model::{TierModelEntry, TierPool, TierSet, label_for};
use crate::tier_definitions::tests::{
    base_checks, blank, checked, custom, detailed, error, labels, ladder, llm, pool, pools, single,
};

fn names(set: &TierSet) -> Vec<String> {
    set.tier_names().into_iter().map(str::to_owned).collect()
}

fn severity(set: &TierSet) -> Vec<&'static str> {
    set.labeled_tiers()
        .into_iter()
        .map(|(tier, _)| tier.name())
        .collect()
}

#[test]
fn a_config_without_custom_tiers_uses_the_built_in_ladder() {
    let set = checked(blank(), &TierChecks::default());
    assert!(set.definitions.is_none());
    assert_eq!(names(&set), ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]);
    assert_eq!(severity(&set), ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]);
    assert!(set.fallback.is_none());
    assert!(!set.non_reasoning);
}

#[test]
fn an_enabled_non_reasoning_tier_leads_the_ladder() {
    let config = section::Config {
        tier_models: pools(&[("NON_REASONING", single("mini"))]),
        non_reasoning_tier: true,
        ..blank()
    };
    let checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        ..base_checks()
    };
    let set = checked(config, &checks);
    assert!(set.non_reasoning);
    assert_eq!(
        severity(&set),
        ["NON_REASONING", "SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]
    );
    assert_eq!(set.tier_names()[0], "NON_REASONING");
    assert_eq!(
        set.pools.get("NON_REASONING").unwrap().models()[0].model_name,
        "mini"
    );
}

#[test]
fn a_custom_ladder_keeps_the_definition_order() {
    let set = checked(custom(), &llm("ECO"));
    assert!(set.definitions.is_some());
    assert_eq!(names(&set), ["ECO", "DEEP"]);
    assert_eq!(set.wire_labels(), ["ECO", "DEEP"]);
    assert_eq!(set.fallback.as_deref(), Some("ECO"));
    assert_eq!(
        set.definitions
            .as_deref()
            .unwrap()
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        ["ECO", "DEEP"]
    );
    assert_eq!(severity(&set), ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]);
}

#[rstest]
#[case::lowercased("eco", Some("ECO"))]
#[case::padded("  eco  ", Some("ECO"))]
#[case::exact("DEEP", Some("DEEP"))]
#[case::unknown("MEDIUM", None)]
#[case::blank_reply("", None)]
fn a_custom_set_resolves_replies_to_its_defined_names(
    #[case] reply: &str,
    #[case] expected: Option<&str>,
) {
    let set = checked(custom(), &llm("ECO"));
    let resolved = set.resolve_classified_tier(reply);
    assert_eq!(resolved.as_ref().map(Tier::name), expected);
}

// the documented casefold deviation: Rust lowercases, so only Python's casefold folds this reply onto the name
#[test]
fn a_reply_that_only_casefold_would_match_does_not_resolve() {
    let config = section::Config {
        tier_definitions: ladder(&[("STRASSE", Some("relays")), ("DEEP", Some("hard"))]),
        tier_models: pools(&[("STRASSE", single("a")), ("DEEP", single("b"))]),
        ..blank()
    };
    let set = checked(config, &llm("STRASSE"));
    assert_eq!(set.resolve_classified_tier("straße"), None);
    assert_eq!(
        set.resolve_classified_tier("STRASSE")
            .as_ref()
            .map(Tier::name),
        Some("STRASSE")
    );
}

#[rstest]
#[case::label("cheap", Some(ComplexityTier::Simple))]
#[case::padded_label("  STANDARD ", Some(ComplexityTier::Medium))]
#[case::canonical("COMPLEX", Some(ComplexityTier::Complex))]
#[case::canonical_of_a_labeled_tier("SIMPLE", Some(ComplexityTier::Simple))]
#[case::lowered_canonical_of_a_labeled_tier("medium", Some(ComplexityTier::Medium))]
#[case::unknown("premium", None)]
fn a_built_in_set_resolves_replies_through_labels(
    #[case] reply: &str,
    #[case] expected: Option<ComplexityTier>,
) {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "Cheap"), ("MEDIUM", "Standard")]),
        ..blank()
    };
    let set = checked(config, &TierChecks::default());
    assert_eq!(
        set.resolve_classified_tier(reply),
        expected.map(Tier::BuiltIn)
    );
    assert_eq!(set.tier_for_label(reply), expected);
}

#[test]
fn display_labels_do_not_rename_the_configuration_keys() {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", "Cheap")]),
        tier_models: pools(&[("SIMPLE", single("small"))]),
        ..blank()
    };
    let set = checked(config, &TierChecks::default());
    assert_eq!(names(&set), ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"]);
    assert!(set.pools.get("SIMPLE").is_some());
    assert!(set.pools.get("Cheap").is_none());
    assert_eq!(
        set.wire_labels(),
        ["Cheap", "MEDIUM", "COMPLEX", "REASONING"]
    );
}

#[test]
fn labeled_tiers_render_the_operator_vocabulary_in_severity_order() {
    let config = section::Config {
        tier_labels: labels(&[("SIMPLE", " Fast "), ("REASONING", "Deep")]),
        non_reasoning_tier: true,
        tier_models: pools(&[("NON_REASONING", single("mini"))]),
        ..blank()
    };
    let checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        ..base_checks()
    };
    let set = checked(config, &checks);
    assert_eq!(
        set.labeled_tiers(),
        [
            (ComplexityTier::NonReasoning, "NON_REASONING"),
            (ComplexityTier::Simple, "Fast"),
            (ComplexityTier::Medium, "MEDIUM"),
            (ComplexityTier::Complex, "COMPLEX"),
            (ComplexityTier::Reasoning, "Deep"),
        ]
    );
    assert_eq!(label_for(&set.labels, ComplexityTier::Simple), "Fast");
    assert_eq!(label_for(&set.labels, ComplexityTier::Medium), "MEDIUM");
}

#[test]
fn a_single_object_entry_keeps_its_request_params() {
    let parameters = Json::Text("xhigh".to_owned());
    let config = section::Config {
        tier_models: pools(&[(
            "REASONING",
            TierPool::Single(detailed(
                "opus",
                &[("reasoning_effort", parameters.clone())],
            )),
        )]),
        ..blank()
    };
    let set = checked(config, &TierChecks::default());
    let reasoning = set.pools.get("REASONING").unwrap();
    assert!(matches!(reasoning, TierPool::Single(_)));
    assert_eq!(reasoning.models().len(), 1);
    assert_eq!(reasoning.models()[0].model_name, "opus");
    assert_eq!(
        reasoning.models()[0].params.get("reasoning_effort"),
        Some(&parameters)
    );
}

#[test]
fn a_pool_keeps_every_entry_with_its_own_params() {
    let config = section::Config {
        tier_models: pools(&[(
            "REASONING",
            TierPool::Pool(vec![
                detailed(
                    "opus",
                    &[("reasoning_effort", Json::Text("xhigh".to_owned()))],
                ),
                section::TierEntry::Model("abc".to_owned()),
            ]),
        )]),
        ..blank()
    };
    let set = checked(config, &TierChecks::default());
    let reasoning = set.pools.get("REASONING").unwrap();
    assert!(!matches!(reasoning, TierPool::Single(_)));
    assert_eq!(reasoning.models().len(), 2);
    assert_eq!(reasoning.models()[0].model_name, "opus");
    assert_eq!(reasoning.models()[1].model_name, "abc");
    assert!(reasoning.models()[1].params.is_empty());
}

#[test]
fn an_absent_pool_map_uses_the_pinned_default_models() {
    let set = checked(blank(), &TierChecks::default());
    assert_eq!(set.pools.len(), 4);
    assert_eq!(
        set.pools.get("SIMPLE").unwrap().models()[0].model_name,
        "gpt-4o-mini"
    );
    assert_eq!(
        set.pools.get("REASONING").unwrap().models()[0].model_name,
        "claude-sonnet-4-20250514"
    );
    let empty = checked(
        section::Config {
            tier_models: Some(BTreeMap::new()),
            ..blank()
        },
        &TierChecks::default(),
    );
    assert!(empty.pools.is_empty());
}

#[rstest]
#[case::plain_pool(pool(&["dup", "dup"]), "SIMPLE")]
#[case::object_and_plain(
    TierPool::Pool(vec![
        detailed("dup", &[("temperature", Json::Number(Number::from(1_u64)))]),
        section::TierEntry::Model("dup".to_owned()),
    ]),
    "SIMPLE"
)]
fn a_repeated_model_name_in_one_pool_is_rejected(#[case] configured: TierPool, #[case] tier: &str) {
    let config = section::Config {
        tier_models: pools(&[(tier, configured)]),
        ..blank()
    };
    assert!(matches!(
        error(config, &TierChecks::default()),
        TierDefinitionsError::DuplicateModelName { tier: reported } if reported == tier
    ));
}

#[test]
fn definitions_are_stripped_and_a_built_in_name_inherits_its_description() {
    let config = section::Config {
        tier_definitions: ladder(&[("  simple  ", Some("   ")), ("  DEEP  ", Some("  hard  "))]),
        tier_models: pools(&[("simple", single("a")), ("DEEP", single("b"))]),
        ..blank()
    };
    let set = checked(config, &llm("simple"));
    let definitions = set.definitions.as_deref().unwrap();
    assert_eq!(definitions[0].name, "simple");
    assert_eq!(definitions[0].description.as_deref(), None);
    assert_eq!(definitions[1].name, "DEEP");
    assert_eq!(definitions[1].description.as_deref(), Some("hard"));
}

#[test]
fn a_section_shaped_rubric_requires_an_llm_classifier() {
    let llm_checks = TierChecks {
        classifier_type: ClassifierKind::Llm,
        classification_prompt: Some("choose a tier"),
        ..base_checks()
    };
    assert!(TierSet::resolve(&blank(), &llm_checks).is_ok());
    let heuristic = TierChecks {
        classifier_type: ClassifierKind::Heuristic,
        classification_examples: Some("one example"),
        ..base_checks()
    };
    assert!(matches!(
        error(blank(), &heuristic),
        TierDefinitionsError::OpeningRequiresClassifier {
            field: "classification_examples",
            ..
        }
    ));
    let whole_prompt = TierChecks {
        classifier_type: ClassifierKind::Llm,
        classification_prompt: Some("choose a tier"),
        llm_system_prompt: Some("whole prompt"),
        ..base_checks()
    };
    assert!(matches!(
        error(blank(), &whole_prompt),
        TierDefinitionsError::OpeningWithSystemPrompt {
            field: "classification_prompt"
        }
    ));
}

#[test]
fn a_blank_label_falls_back_to_the_canonical_name() {
    let labels = BTreeMap::from([(ComplexityTier::Simple, "   ".to_owned())]);
    assert_eq!(label_for(&labels, ComplexityTier::Simple), "SIMPLE");
    assert_eq!(label_for(&labels, ComplexityTier::Medium), "MEDIUM");
}

#[test]
fn entry_params_stay_typed_json_values() {
    let config = section::Config {
        tier_models: pools(&[(
            "MEDIUM",
            TierPool::Single(detailed(
                "mid",
                &[
                    ("max_tokens", Json::Number(Number::from(512_u64))),
                    ("top_p", Json::Number(Number::from(1_u64))),
                ],
            )),
        )]),
        ..blank()
    };
    let set = checked(config, &TierChecks::default());
    let entry: &TierModelEntry = &set.pools.get("MEDIUM").unwrap().models()[0];
    assert_eq!(entry.params.len(), 2);
    assert_eq!(
        entry.params.get("max_tokens"),
        Some(&Json::Number(Number::from(512_u64)))
    );
}
