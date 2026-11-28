use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions::{TierDefinition, TierParam};
use betterlite_router::complexity_config::checks::Feature;
use betterlite_router::complexity_scoring::decision::RoutingDecision;
use betterlite_router::router_predicates::JsonMap;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};
use indexmap::IndexMap;
use rstest::rstest;

use crate::routing::cache_alternatives::config::CacheRoutingConfig;
use crate::routing::cache_alternatives::eligibility::{eligible_models, supported_router_marker};
use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::tests::support::{
    Fixture, decision, json, ladder, marker_deployment, marker_params,
};

fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(tier, model)| ((*tier).to_owned(), (*model).to_owned()))
        .collect()
}

fn single_tier(pool: TierPool) -> TierSet {
    TierSet {
        definitions: None,
        pools: BTreeMap::from([("SIMPLE".to_owned(), pool)]),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

fn entry(model: &str, params: IndexMap<String, TierParam>) -> TierModelEntry {
    TierModelEntry {
        model_name: model.to_owned(),
        params,
    }
}

fn classified() -> RoutingDecision {
    decision(&[("tier", "SIMPLE"), ("cause", "heuristic_scorer")])
}

#[test]
fn eligible_models_keeps_tiers_at_or_above_the_decision_tier() {
    let tiers = ladder(&[
        ("SIMPLE", "cheap"),
        ("MEDIUM", "cold"),
        ("COMPLEX", "strong"),
        ("REASONING", "strong"),
    ]);
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    let top = eligible_models(&config, &classified());
    assert_eq!(
        top,
        pairs(&[
            ("SIMPLE", "cheap"),
            ("MEDIUM", "cold"),
            ("COMPLEX", "strong")
        ])
    );
    let floor = eligible_models(
        &config,
        &decision(&[("tier", "MEDIUM"), ("cause", "heuristic_scorer")]),
    );
    assert_eq!(floor, pairs(&[("MEDIUM", "cold"), ("COMPLEX", "strong")]));
}

#[test]
fn eligible_models_is_empty_beyond_the_ladder_or_without_a_decision_tier() {
    let tiers = ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]);
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    let empty = Vec::new();
    assert_eq!(
        eligible_models(
            &config,
            &decision(&[("tier", "REASONING"), ("cause", "heuristic_scorer")])
        ),
        empty
    );
    assert_eq!(
        eligible_models(
            &config,
            &decision(&[("tier", "TINY"), ("cause", "heuristic_scorer")])
        ),
        empty
    );
    assert_eq!(
        eligible_models(&config, &decision(&[("cause", "heuristic_scorer")])),
        empty
    );
    assert_eq!(
        eligible_models(&config, &decision(&[("tier", "SIMPLE")])),
        empty
    );
}

#[rstest]
#[case::heuristic_scorer("heuristic_scorer")]
#[case::heuristic_v2("heuristic_v2")]
#[case::reasoning_override("reasoning_override")]
#[case::llm_classifier("llm_classifier")]
#[case::llm_v2_classifier("llm_v2_classifier")]
#[case::jev_classifier("jev_classifier")]
#[case::capability_classifier("capability_classifier")]
#[case::heuristic_first_short_circuit("heuristic_first_short_circuit")]
#[case::hybrid_short_circuit("hybrid_short_circuit")]
#[case::classifier_plugin("classifier_plugin")]
fn every_classified_cause_may_compare_cache_costs(#[case] cause: &str) {
    let tiers = ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]);
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    let got = eligible_models(&config, &decision(&[("tier", "SIMPLE"), ("cause", cause)]));
    assert_eq!(
        got,
        pairs(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]),
        "cause {cause}"
    );
}

#[rstest]
#[case::prompt_cache_cost("prompt_cache_cost")]
#[case::literal_keyword_match("literal_keyword_match")]
#[case::session_affinity_pin("session_affinity_pin")]
#[case::classifier_fallback("classifier_fallback")]
#[case::default_fallback("default_fallback")]
fn unclassified_causes_keep_the_classified_model(#[case] cause: &str) {
    let tiers = ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]);
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    let got = eligible_models(&config, &decision(&[("tier", "SIMPLE"), ("cause", cause)]));
    assert!(got.is_empty(), "cause {cause}");
}

#[test]
fn repeated_model_in_multiple_tiers_is_only_considered_once() {
    let tiers = ladder(&[
        ("SIMPLE", "cheap"),
        ("MEDIUM", "strong"),
        ("COMPLEX", "strong"),
    ]);
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    let got = eligible_models(&config, &classified());
    assert_eq!(got, pairs(&[("SIMPLE", "cheap"), ("MEDIUM", "strong")]));
}

#[test]
fn custom_tier_definitions_keep_cache_costs_out() {
    let mut tiers = ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]);
    tiers.definitions = Some(vec![TierDefinition {
        name: "LOW".to_owned(),
        description: None,
    }]);
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    assert!(eligible_models(&config, &classified()).is_empty());
}

#[rstest]
#[case::adaptive(Feature::Adaptive)]
#[case::plugins(Feature::Plugins)]
#[case::session_affinity(Feature::SessionAffinity)]
#[case::user_turn_classification(Feature::UserTurnClassification)]
fn pinned_or_adaptive_policies_are_preserved(#[case] feature: Feature) {
    let tiers = ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]);
    let features = [feature];
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &features,
    };
    assert!(
        eligible_models(&config, &classified()).is_empty(),
        "feature {feature:?}"
    );
}

#[test]
fn unrelated_features_do_not_keep_cache_costs_out() {
    let tiers = ladder(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")]);
    let features = [Feature::StallEscalation, Feature::SemanticKeywordMatching];
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &features,
    };
    assert_eq!(
        eligible_models(&config, &classified()),
        pairs(&[("SIMPLE", "cheap"), ("COMPLEX", "strong")])
    );
}

#[test]
fn tier_litellm_params_keep_cache_costs_out() {
    let params = IndexMap::from([("rpm".to_owned(), TierParam::Value(json("5")))]);
    let tiers = single_tier(TierPool::Single(entry("cheap", params)));
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    assert!(eligible_models(&config, &classified()).is_empty());
}

#[test]
fn a_tier_pool_keeps_cache_costs_out() {
    let tiers = single_tier(TierPool::Pool(vec![
        entry("cheap", IndexMap::new()),
        entry("cold", IndexMap::new()),
    ]));
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    assert!(eligible_models(&config, &classified()).is_empty());
}

#[test]
fn a_plain_single_tier_entry_is_eligible() {
    let tiers = single_tier(TierPool::Single(entry("cheap", IndexMap::new())));
    let config = CacheRoutingConfig {
        tiers: &tiers,
        features: &[],
    };
    assert_eq!(
        eligible_models(&config, &classified()),
        pairs(&[("SIMPLE", "cheap")])
    );
}

#[test]
fn supported_router_marker_accepts_a_bare_marker_with_allowed_options() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    assert!(supported_router_marker(&host, "smart", &JsonMap::new()).unwrap());
    let params = r#"{"model":"auto_router/complexity_router","complexity_router_config":{"tiers":{"SIMPLE":"cheap"}},"rpm":5,"tpm":1000,"tags":["a"],"timeout":2,"stream_timeout":3,"num_retries":1,"use_in_pass_through":false}"#;
    host.deployments
        .insert("smart".to_owned(), vec![marker_deployment(params)]);
    assert!(supported_router_marker(&host, "smart", &JsonMap::new()).unwrap());
}

#[test]
fn supported_router_marker_rejects_foreign_options_or_models() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    let extra = r#"{"model":"auto_router/complexity_router","temperature":0.1}"#;
    host.deployments
        .insert("smart".to_owned(), vec![marker_deployment(extra)]);
    assert!(!supported_router_marker(&host, "smart", &JsonMap::new()).unwrap());
    let foreign = marker_params().replace(
        "auto_router/complexity_router",
        "anthropic/claude-haiku-4-5",
    );
    host.deployments
        .insert("smart".to_owned(), vec![marker_deployment(&foreign)]);
    assert!(!supported_router_marker(&host, "smart", &JsonMap::new()).unwrap());
}

#[test]
fn supported_router_marker_rejects_an_alias_without_deployments() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(!supported_router_marker(&host, "absent", &JsonMap::new()).unwrap());
}

#[test]
fn supported_router_marker_reports_a_deployment_lookup_error() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.missing.push("smart".to_owned());
    let error = supported_router_marker(&host, "smart", &JsonMap::new()).unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Deployments { .. }));
}
