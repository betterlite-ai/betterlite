use std::collections::BTreeSet;

use betterlite_config::sections::adaptive_classification::Weights;
use betterlite_config::sections::complexity_adaptive_placement as placement_section;
use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::adaptive_classification::config::{AdaptiveRouterPreferences, AdaptiveRouterWeights};
use crate::adaptive_classification::constants::DEFAULT_COST_WEIGHT as ROUTER_DEFAULT_COST_WEIGHT;
use crate::adaptive_classification::request_type::RequestType;
use crate::adaptive_selection::default_preferences;
use crate::bandit_math::initial_cell;
use crate::complexity_adaptive_placement::PlacementSnapshot;
use crate::complexity_adaptive_placement::candidates::CandidateQuery;
use crate::complexity_adaptive_placement::decision::{
    AdaptiveChoice, AdaptiveDecision, CandidateScore, ColdStartCandidate, ColdStartDecision,
};
use crate::complexity_adaptive_placement::facts::ModelFacts;
use crate::complexity_adaptive_placement::pick::Placed;
use crate::complexity_adaptive_placement::settings::PlacementSettings;

use super::support::{
    FactSpec, Harness, POOLS, SEED, World, adaptive_service, adaptive_settings, cell_in,
    configured_settings, cost, model_facts, priced, request, tier, warm,
};

#[test]
fn the_construction_uses_the_pool_models_with_pinned_priors_and_costs() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let router =
        adaptive_service(adaptive_settings(true), POOLS, &facts, &[]).expect("the router builds");
    let snapshot = router.snapshot().expect("adaptive builds a snapshot");
    assert_eq!(snapshot.available_models, vec!["mini", "mid", "big"]);
    assert_eq!(
        snapshot.model_tiers,
        vec![
            ("mini".to_owned(), vec!["SIMPLE".to_owned()]),
            ("mid".to_owned(), vec!["MEDIUM".to_owned()]),
            ("big".to_owned(), vec!["COMPLEX".to_owned()]),
        ]
    );
    assert_eq!(snapshot.preferences, pinned_prior_preferences());
    assert_eq!(
        snapshot.costs,
        vec![
            ("mini".to_owned(), cost(1)),
            ("mid".to_owned(), cost(2)),
            ("big".to_owned(), cost(3)),
        ]
    );
    assert_eq!(
        snapshot.weights,
        AdaptiveRouterWeights {
            quality: 0.3,
            economy: 0.7,
        }
    );
    assert_eq!(snapshot.tier_distance_penalty.to_bits(), 0.5_f64.to_bits());
    assert_eq!(snapshot.eligible, AdaptiveEligible::All);
    assert_eq!(snapshot.cells.len(), 21);
    let cell = cell_in(&snapshot, RequestType::General, "big");
    assert_eq!(cell.alpha.to_bits(), 5.0_f64.to_bits());
    assert_eq!(cell.beta.to_bits(), 5.0_f64.to_bits());
    assert_eq!(cell.samples, 0);
}

fn pinned_prior_preferences() -> Vec<(String, AdaptiveRouterPreferences)> {
    let pinned = AdaptiveRouterPreferences {
        quality_tier: 2,
        strengths: Vec::new(),
    };
    vec![
        ("mini".to_owned(), pinned.clone()),
        ("mid".to_owned(), pinned.clone()),
        ("big".to_owned(), pinned),
    ]
}

#[test]
fn the_pinned_defaults_apply_when_the_deployment_declares_nothing() {
    let resolved = PlacementSettings::resolve(&placement_section::Config::default())
        .expect("the pinned defaults are valid");
    assert!(!resolved.adaptive);
    assert_eq!(
        resolved.weights,
        AdaptiveRouterWeights {
            quality: 0.3,
            economy: 0.7,
        }
    );
    assert_eq!(
        resolved.weights.quality.to_bits(),
        placement_section::DEFAULT_QUALITY_WEIGHT.to_bits()
    );
    assert_eq!(
        resolved.weights.economy.to_bits(),
        placement_section::DEFAULT_COST_WEIGHT.to_bits()
    );
    assert_eq!(
        resolved.tier_distance_penalty.to_bits(),
        placement_section::DEFAULT_TIER_DISTANCE_PENALTY.to_bits()
    );
    assert_eq!(resolved.eligible, AdaptiveEligible::All);
}

#[test]
fn declared_weights_penalty_and_eligible_mode_replace_the_defaults() {
    let resolved = PlacementSettings::resolve(&placement_section::Config {
        adaptive: true,
        weights: Some(Weights {
            quality: Some(0.4_f64),
            economy: Some(0.6_f64),
        }),
        tier_distance_penalty: 0.25,
        eligible: AdaptiveEligible::ClassifiedTier,
    })
    .expect("the declared settings are valid");
    assert!(resolved.adaptive);
    assert_eq!(
        resolved.weights,
        AdaptiveRouterWeights {
            quality: 0.4,
            economy: 0.6,
        }
    );
    assert_eq!(resolved.tier_distance_penalty.to_bits(), 0.25_f64.to_bits());
    assert_eq!(resolved.eligible, AdaptiveEligible::ClassifiedTier);
}

#[test]
fn a_declared_weights_object_keeps_its_field_defaults() {
    let resolved = PlacementSettings::resolve(&placement_section::Config {
        adaptive: false,
        weights: Some(Weights {
            quality: Some(0.9_f64),
            economy: None,
        }),
        tier_distance_penalty: placement_section::DEFAULT_TIER_DISTANCE_PENALTY,
        eligible: AdaptiveEligible::All,
    })
    .expect("a lone quality weight is valid");
    assert_eq!(resolved.weights.quality.to_bits(), 0.9_f64.to_bits());
    assert_eq!(
        resolved.weights.economy.to_bits(),
        ROUTER_DEFAULT_COST_WEIGHT.to_bits()
    );
    assert!(!resolved.adaptive);
}

// {} keeps AdaptiveRouterWeights' own 0.7/0.3, unlike an absent key's 0.3/0.7
#[test]
fn a_declared_empty_weights_object_takes_the_adaptive_router_defaults() {
    let resolved = PlacementSettings::resolve(&placement_section::Config {
        adaptive: true,
        weights: Some(Weights {
            quality: None,
            economy: None,
        }),
        tier_distance_penalty: placement_section::DEFAULT_TIER_DISTANCE_PENALTY,
        eligible: AdaptiveEligible::All,
    })
    .expect("an empty weights object is valid");
    assert_eq!(
        resolved.weights,
        AdaptiveRouterWeights {
            quality: 0.7,
            economy: 0.3,
        }
    );
}

#[test]
fn declared_preferences_shape_the_cold_start_priors() {
    let facts = declared_preference_facts();
    let router =
        adaptive_service(adaptive_settings(true), POOLS, &facts, &[]).expect("the router builds");
    let snapshot = router.snapshot().expect("adaptive builds a snapshot");
    assert_eq!(
        snapshot.preferences,
        vec![
            (
                "mini".to_owned(),
                AdaptiveRouterPreferences {
                    quality_tier: 3,
                    strengths: Vec::new(),
                },
            ),
            (
                "mid".to_owned(),
                AdaptiveRouterPreferences {
                    quality_tier: 2,
                    strengths: vec![RequestType::General],
                },
            ),
            ("big".to_owned(), default_preferences()),
        ]
    );
    declared_prior_cells(&snapshot);
}

fn declared_preference_facts() -> ModelFacts {
    model_facts(&[
        FactSpec {
            model: "mini",
            preferences: Some((3, &[])),
            params_cost: Some(cost(1)),
            info_cost: None,
        },
        FactSpec {
            model: "mid",
            preferences: Some((2, &[RequestType::General])),
            params_cost: Some(cost(2)),
            info_cost: None,
        },
        FactSpec {
            model: "big",
            preferences: None,
            params_cost: None,
            info_cost: None,
        },
    ])
}

fn declared_prior_cells(snapshot: &PlacementSnapshot) {
    let mini = cell_in(snapshot, RequestType::General, "mini");
    let mini_prior = initial_cell(
        &AdaptiveRouterPreferences {
            quality_tier: 3,
            strengths: Vec::new(),
        },
        RequestType::General,
    )
    .expect("tier 3 has a prior");
    assert_eq!(mini.alpha.to_bits(), mini_prior.alpha.to_bits());
    let mid = cell_in(snapshot, RequestType::General, "mid");
    let mid_prior = initial_cell(
        &AdaptiveRouterPreferences {
            quality_tier: 2,
            strengths: vec![RequestType::General],
        },
        RequestType::General,
    )
    .expect("tier 2 has a prior");
    assert_eq!(mid.alpha.to_bits(), mid_prior.alpha.to_bits());
    let mid_writing = cell_in(snapshot, RequestType::Writing, "mid");
    let writing_prior = initial_cell(&default_preferences(), RequestType::Writing)
        .expect("the default tier has a prior");
    assert_eq!(mid_writing.alpha.to_bits(), writing_prior.alpha.to_bits());
}

#[test]
fn the_first_deployment_declares_the_price_and_params_win_over_model_info() {
    let facts = model_facts(&[
        FactSpec {
            model: "mini",
            preferences: None,
            params_cost: None,
            info_cost: Some(cost(5)),
        },
        FactSpec {
            model: "mini",
            preferences: None,
            params_cost: Some(cost(1)),
            info_cost: Some(cost(9)),
        },
        FactSpec {
            model: "mid",
            preferences: None,
            params_cost: Some(cost(2)),
            info_cost: Some(cost(7)),
        },
        FactSpec {
            model: "big",
            preferences: None,
            params_cost: None,
            info_cost: None,
        },
    ]);
    assert_eq!(facts.costs.get("mini"), Some(&cost(5)));
    assert_eq!(facts.costs.get("mid"), Some(&cost(2)));
    assert_eq!(facts.costs.get("big"), Some(&Decimal::ZERO));
}

#[test]
fn a_cold_classified_pool_places_from_that_pool_and_records_the_decision() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness =
        Harness::new(adaptive_settings(true), POOLS, &facts, &[], None).expect("the router builds");
    let classified = tier("SIMPLE");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed, expected_cold_start_placement());
    let Some(Json::Object(decision)) = harness.decision() else {
        panic!("the decision is recorded as an object");
    };
    assert_cold_start_fields(decision);
    assert_cold_start_candidates(decision);
}

fn expected_cold_start_placement() -> Placed {
    Placed {
        model: "mini".to_owned(),
        decision: Some(AdaptiveDecision::ColdStart(ColdStartDecision {
            classified_tier: "SIMPLE".to_owned(),
            request_type: RequestType::General,
            weights: AdaptiveRouterWeights {
                quality: 0.3,
                economy: 0.7,
            },
            tier_distance_penalty: 0.5,
            chosen_model: "mini".to_owned(),
            candidates: vec![ColdStartCandidate {
                model: "mini".to_owned(),
                total_samples: 0,
            }],
        })),
    }
}

fn assert_cold_start_fields(decision: &IndexMap<String, Json>) {
    assert_eq!(
        decision.get("phase"),
        Some(&Json::Text("cold_start".to_owned()))
    );
    assert_eq!(
        decision.get("eligible_mode"),
        Some(&Json::Text("classified_tier".to_owned()))
    );
    assert_eq!(
        decision.get("request_type"),
        Some(&Json::Text("general".to_owned()))
    );
    assert_eq!(
        decision.get("chosen_model"),
        Some(&Json::Text("mini".to_owned()))
    );
    assert_eq!(
        decision.get("quality_weight"),
        Some(&Json::Number(Number::from_f64(0.3).expect("0.3 is finite")))
    );
    assert_eq!(
        decision.get("cost_weight"),
        Some(&Json::Number(Number::from_f64(0.7).expect("0.7 is finite")))
    );
    assert_eq!(
        decision.get("tier_distance_penalty"),
        Some(&Json::Number(Number::from_f64(0.5).expect("0.5 is finite")))
    );
    assert_eq!(
        decision.get("classified_tier"),
        Some(&Json::Text("SIMPLE".to_owned()))
    );
}

fn assert_cold_start_candidates(decision: &IndexMap<String, Json>) {
    let Some(Json::List(candidates)) = decision.get("candidates") else {
        panic!("the decision lists its candidates");
    };
    assert_eq!(
        *candidates,
        vec![Json::Object(IndexMap::from_iter([
            ("model".to_owned(), Json::Text("mini".to_owned())),
            (
                "total_samples".to_owned(),
                Json::Number(Number::from(0_u64)),
            ),
        ]))]
    );
}

#[test]
fn a_custom_tier_name_falls_back_to_the_default_model() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(
        adaptive_settings(true),
        POOLS,
        &facts,
        &[],
        Some("openai/fallback"),
    )
    .expect("the router builds");
    let classified = tier("legacy");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(
        placed,
        Placed {
            model: "openai/fallback".to_owned(),
            decision: None,
        }
    );
    assert_eq!(harness.decision(), None);
}

// a custom ladder builds where LiteLLM raises ValueError; its classified tier still takes the fallback
#[test]
fn an_adaptive_custom_ladder_builds_and_keeps_the_named_tier_fallback() {
    let pools: &[(&str, &[&str])] = &[("basic", &["mini"]), ("advanced", &["big"])];
    let facts = priced(&[("mini", 1), ("big", 2)]);
    let mut harness = Harness::new(
        adaptive_settings(true),
        pools,
        &facts,
        &[],
        Some("openai/fallback"),
    )
    .expect("an adaptive router over a custom ladder builds");
    assert!(harness.router.snapshot().is_some());
    let classified = tier("advanced");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.decision, None);
}

#[test]
fn adaptive_off_places_the_tier_fallback() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(adaptive_settings(false), POOLS, &facts, &[], None)
        .expect("the router builds without state");
    assert_eq!(harness.router.snapshot(), None);
    let classified = tier("MEDIUM");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(
        placed,
        Placed {
            model: "mid".to_owned(),
            decision: None,
        }
    );
    assert_eq!(harness.decision(), None);
}

#[test]
fn the_distance_penalty_keeps_the_nearer_tier_against_a_higher_sample() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(
        adaptive_settings(true),
        POOLS,
        &facts,
        &[0.9_f64, 0.5_f64, 0.95_f64],
        None,
    )
    .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mid");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_distance_candidates(choice);
    assert_distance_scores(choice);
    let Some(Json::Object(decision)) = harness.decision() else {
        panic!("the decision is recorded as an object");
    };
    assert_rendered_decision(decision);
    assert_rendered_candidates(decision, choice);
}

fn candidate_rendering(candidate: &CandidateScore) -> Json {
    Json::Object(IndexMap::from([
        ("model".to_owned(), Json::Text(candidate.model.clone())),
        (
            "quality_sample".to_owned(),
            Json::Number(Number::from_f64(candidate.quality_sample).expect("a finite sample")),
        ),
        (
            "cost_score".to_owned(),
            Json::Number(Number::from_f64(candidate.economy_score).expect("a finite score")),
        ),
        (
            "tier_distance".to_owned(),
            Json::Number(Number::from(candidate.tier_distance)),
        ),
        (
            "score".to_owned(),
            Json::Number(Number::from_f64(candidate.score).expect("a finite score")),
        ),
    ]))
}

fn assert_distance_candidates(choice: &AdaptiveChoice) {
    assert_eq!(choice.classified_tier, "MEDIUM");
    assert_eq!(choice.eligible, AdaptiveEligible::All);
    let models: Vec<&str> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.model.as_str())
        .collect();
    assert_eq!(models, vec!["mini", "mid", "big"]);
    let distances: Vec<u32> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.tier_distance)
        .collect();
    assert_eq!(distances, vec![1, 0, 1]);
    let samples: Vec<u64> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.quality_sample.to_bits())
        .collect();
    assert_eq!(
        samples,
        vec![0.9_f64.to_bits(), 0.5_f64.to_bits(), 0.95_f64.to_bits()]
    );
}

fn assert_distance_scores(choice: &AdaptiveChoice) {
    let costs: Vec<u64> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.economy_score.to_bits())
        .collect();
    assert_eq!(
        costs,
        vec![1.0_f64.to_bits(), 0.5_f64.to_bits(), 0.0_f64.to_bits()]
    );
    let scores: Vec<f64> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.score)
        .collect();
    assert!(scores[1] > scores[0]);
    assert!(scores[1] > scores[2]);
}

fn assert_rendered_decision(decision: &IndexMap<String, Json>) {
    assert_eq!(
        decision.get("phase"),
        Some(&Json::Text("adaptive".to_owned()))
    );
    assert_eq!(
        decision.get("eligible_mode"),
        Some(&Json::Text("all".to_owned()))
    );
    assert_eq!(
        decision.get("classified_tier"),
        Some(&Json::Text("MEDIUM".to_owned()))
    );
    assert_eq!(
        decision.get("request_type"),
        Some(&Json::Text("general".to_owned()))
    );
    assert_eq!(
        decision.get("quality_weight"),
        Some(&Json::Number(Number::from_f64(0.3).expect("0.3 is finite")))
    );
    assert_eq!(
        decision.get("cost_weight"),
        Some(&Json::Number(Number::from_f64(0.7).expect("0.7 is finite")))
    );
    assert_eq!(
        decision.get("tier_distance_penalty"),
        Some(&Json::Number(Number::from_f64(0.5).expect("0.5 is finite")))
    );
    assert_eq!(
        decision.get("chosen_model"),
        Some(&Json::Text("mid".to_owned()))
    );
}

fn assert_rendered_candidates(decision: &IndexMap<String, Json>, choice: &AdaptiveChoice) {
    let Some(Json::List(candidates)) = decision.get("candidates") else {
        panic!("the decision lists its candidates");
    };
    let expected: Vec<Json> = choice.candidates.iter().map(candidate_rendering).collect();
    assert_eq!(candidates, &expected);
}

// the declared weights reach placement_score: the same samples under the pinned defaults keep mini
#[test]
fn declared_weights_decide_the_adaptive_pick() {
    let facts = priced(&[("mini", 1), ("mid", 5), ("big", 3)]);
    let mut configured = placement_harness(
        quality_only_settings(),
        &facts,
        &[0.5_f64, 0.9_f64, 0.0_f64],
    );
    warm(&mut configured.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let placed = configured
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mid");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_eq!(
        choice.weights,
        AdaptiveRouterWeights {
            quality: 1.0,
            economy: 0.0,
        }
    );
    assert_eq!(
        choice.tier_distance_penalty.to_bits(),
        placement_section::DEFAULT_TIER_DISTANCE_PENALTY.to_bits()
    );
    let mut defaults = placement_harness(
        adaptive_settings(true),
        &facts,
        &[0.5_f64, 0.9_f64, 0.0_f64],
    );
    warm(&mut defaults.router, RequestType::General, &["mid"]);
    let default_placed = defaults
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(default_placed.model, "mini");
}

fn quality_only_settings() -> PlacementSettings {
    PlacementSettings::resolve(&placement_section::Config {
        adaptive: true,
        weights: Some(Weights {
            quality: Some(1.0_f64),
            economy: Some(0.0_f64),
        }),
        tier_distance_penalty: placement_section::DEFAULT_TIER_DISTANCE_PENALTY,
        eligible: AdaptiveEligible::All,
    })
    .expect("the declared weights are valid")
}

fn placement_harness(settings: PlacementSettings, facts: &ModelFacts, samples: &[f64]) -> Harness {
    Harness::new(settings, POOLS, facts, samples, None).expect("the router builds")
}

// the declared penalty reaches placement_score: the same samples under the pinned 0.5 keep mid
#[test]
fn a_declared_distance_penalty_decides_the_adaptive_pick() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut configured = Harness::new(
        configured_settings(true, 0.0, AdaptiveEligible::All),
        POOLS,
        &facts,
        &[0.0_f64, 0.5_f64, 0.0_f64],
        None,
    )
    .expect("the router builds");
    warm(&mut configured.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let placed = configured
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mini");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_eq!(choice.tier_distance_penalty.to_bits(), 0.0_f64.to_bits());
    let mut defaults = Harness::new(
        adaptive_settings(true),
        POOLS,
        &facts,
        &[0.0_f64, 0.5_f64, 0.0_f64],
        None,
    )
    .expect("the router builds");
    warm(&mut defaults.router, RequestType::General, &["mid"]);
    let default_placed = defaults
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(default_placed.model, "mid");
}

#[test]
fn classified_tier_mode_samples_only_its_pool_with_no_distance() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(
        configured_settings(true, 0.5, AdaptiveEligible::ClassifiedTier),
        POOLS,
        &facts,
        &[0.4_f64],
        None,
    )
    .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mid");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_eq!(choice.eligible, AdaptiveEligible::ClassifiedTier);
    assert_eq!(choice.candidates.len(), 1);
    assert_eq!(choice.candidates[0].model, "mid");
    assert_eq!(choice.candidates[0].tier_distance, 0);
    assert_eq!(
        choice.candidates[0].quality_sample.to_bits(),
        0.4_f64.to_bits()
    );
}

#[test]
fn a_hard_floor_keeps_the_models_at_or_above_it() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(
        adaptive_settings(true),
        POOLS,
        &facts,
        &[0.9_f64, 0.1_f64],
        None,
    )
    .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let floor = tier("MEDIUM");
    let mut bounded = request(&classified, "hello");
    bounded.hard_floor = Some(&floor);
    let placed = harness.pick(&bounded).expect("the pick succeeds");
    assert_eq!(placed.model, "mid");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    let models: Vec<&str> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.model.as_str())
        .collect();
    assert_eq!(models, vec!["mid", "big"]);
}

#[test]
fn a_hard_ceiling_keeps_the_models_at_or_below_it() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(adaptive_settings(true), POOLS, &facts, &[0.2_f64], None)
        .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let ceiling = tier("SIMPLE");
    let mut bounded = request(&classified, "hello");
    bounded.hard_ceiling = Some(&ceiling);
    let placed = harness.pick(&bounded).expect("the pick succeeds");
    assert_eq!(placed.model, "mini");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_eq!(choice.candidates.len(), 1);
    assert_eq!(choice.candidates[0].model, "mini");
    assert_eq!(choice.candidates[0].tier_distance, 1);
}

#[test]
fn a_model_in_two_pools_passes_a_bound_through_either_membership() {
    let pools: &[(&str, &[&str])] = &[
        ("SIMPLE", &["mini"]),
        ("MEDIUM", &["mid"]),
        ("COMPLEX", &["mini", "big"]),
    ];
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let router =
        adaptive_service(adaptive_settings(true), pools, &facts, &[]).expect("the router builds");
    let classified = tier("MEDIUM");
    let floor = tier("COMPLEX");
    let query = CandidateQuery {
        classified_tier: &classified,
        hard_floor: Some(&floor),
        hard_ceiling: None,
        fit_filter: None,
    };
    assert_eq!(
        router.candidate_models(&query).expect("the query resolves"),
        vec!["mini", "big"]
    );
}

#[test]
fn normalization_uses_the_broader_candidate_set() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(adaptive_settings(true), POOLS, &facts, &[0.5_f64], None)
        .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let floor = tier("COMPLEX");
    let mut bounded = request(&classified, "hello");
    bounded.hard_floor = Some(&floor);
    let placed = harness.pick(&bounded).expect("the pick succeeds");
    assert_eq!(placed.model, "big");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_eq!(choice.candidates.len(), 1);
    assert_eq!(choice.candidates[0].model, "big");
    assert_eq!(
        choice.candidates[0].economy_score.to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(choice.candidates[0].tier_distance, 1);
}

#[test]
fn the_context_filter_narrows_the_cold_start_pool() {
    let pools: &[(&str, &[&str])] = &[("SIMPLE", &["mini", "small"]), ("MEDIUM", &["mid"])];
    let facts = priced(&[("mini", 1), ("small", 1), ("mid", 2)]);
    let mut harness =
        Harness::new(adaptive_settings(true), pools, &facts, &[], None).expect("the router builds");
    let fit = BTreeSet::from(["small".to_owned()]);
    let classified = tier("SIMPLE");
    let mut filtered = request(&classified, "hello");
    filtered.fit_filter = Some(&fit);
    let placed = harness.pick(&filtered).expect("the pick succeeds");
    assert_eq!(placed.model, "small");
    let Some(AdaptiveDecision::ColdStart(decision)) = &placed.decision else {
        panic!("the cold classified pool places");
    };
    assert_eq!(
        decision.candidates,
        vec![ColdStartCandidate {
            model: "small".to_owned(),
            total_samples: 0,
        }]
    );
}

#[test]
fn a_bound_that_excludes_every_fitting_model_falls_back_to_the_fitting_pool() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness =
        Harness::new(adaptive_settings(true), POOLS, &facts, &[], None).expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mini"]);
    let fit = BTreeSet::from(["mini".to_owned()]);
    let classified = tier("SIMPLE");
    let floor = tier("COMPLEX");
    let mut filtered = request(&classified, "hello");
    filtered.fit_filter = Some(&fit);
    filtered.hard_floor = Some(&floor);
    let placed = harness.pick(&filtered).expect("the pick succeeds");
    assert_eq!(
        placed,
        Placed {
            model: "mini".to_owned(),
            decision: None,
        }
    );
}

#[test]
fn a_filter_that_matches_no_pool_model_places_the_tier() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness =
        Harness::new(adaptive_settings(true), POOLS, &facts, &[], None).expect("the router builds");
    let fit = BTreeSet::from(["ghost".to_owned()]);
    let classified = tier("MEDIUM");
    let mut filtered = request(&classified, "hello");
    filtered.fit_filter = Some(&fit);
    let placed = harness.pick(&filtered).expect("the pick succeeds");
    assert_eq!(
        placed,
        Placed {
            model: "mid".to_owned(),
            decision: None,
        }
    );
}

// every draw the tier pool would make is over both models, so a fitting pick keeps choosing small
#[rstest]
fn the_context_filter_overrides_the_tier_draw(
    #[values(0_u64, 1_u64, 2_u64, 3_u64, 4_u64, 5_u64, 6_u64, 7_u64)] seed: u64,
) {
    let pools: &[(&str, &[&str])] = &[("SIMPLE", &["mini", "small"])];
    let facts = priced(&[("mini", 1), ("small", 1)]);
    let mut harness = Harness::new(adaptive_settings(false), pools, &facts, &[], None)
        .expect("the router builds without state");
    harness.reseed(seed);
    let fit = BTreeSet::from(["small".to_owned()]);
    let classified = tier("SIMPLE");
    let mut filtered = request(&classified, "hello");
    filtered.fit_filter = Some(&fit);
    let placed = harness.pick(&filtered).expect("the pick succeeds");
    assert_eq!(placed.model, "small");
}

#[test]
fn an_observation_wakes_a_cold_cell_for_the_next_pick() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(
        adaptive_settings(true),
        POOLS,
        &facts,
        &[0.5_f64, 0.5_f64, 0.5_f64],
        None,
    )
    .expect("the router builds");
    let classified = tier("MEDIUM");
    let first = harness
        .pick(&request(&classified, "hello"))
        .expect("the cold pick succeeds");
    assert_eq!(first.model, "mid");
    assert!(matches!(
        first.decision,
        Some(AdaptiveDecision::ColdStart(_))
    ));
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let second = harness
        .pick(&request(&classified, "hello"))
        .expect("the adaptive pick succeeds");
    let Some(AdaptiveDecision::Adaptive(choice)) = &second.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    assert_eq!(choice.candidates.len(), 3);
}

#[test]
fn a_request_without_kwargs_still_places_and_records_nothing() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut router =
        adaptive_service(adaptive_settings(true), POOLS, &facts, &[]).expect("the router builds");
    let world = World::new(POOLS, None);
    let context = world.context();
    let placement = world.placement(&context);
    let classified = tier("SIMPLE");
    let mut random = StdRng::seed_from_u64(SEED);
    let placed = router
        .pick(
            &request(&classified, "hello"),
            &placement,
            None,
            &mut random,
        )
        .expect("the pick succeeds without kwargs");
    assert_eq!(placed.model, "mini");
    assert!(matches!(
        placed.decision,
        Some(AdaptiveDecision::ColdStart(_))
    ));
}

#[test]
fn the_decision_merges_into_existing_request_metadata() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness =
        Harness::new(adaptive_settings(true), POOLS, &facts, &[], None).expect("the router builds");
    harness.kwargs.insert(
        "metadata".to_owned(),
        Json::Object(IndexMap::from_iter([(
            "trace_id".to_owned(),
            Json::Text("abc".to_owned()),
        )])),
    );
    let classified = tier("SIMPLE");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mini");
    let Some(Json::Object(metadata)) = harness.kwargs.get("metadata") else {
        panic!("metadata stays an object");
    };
    assert_eq!(
        metadata.get("trace_id"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert!(metadata.contains_key("adaptive_router_decision"));
}

// B374 keeps the pools sorted, so the union follows severity order whatever the YAML order was
#[test]
fn available_models_follow_severity_order_not_declaration_order() {
    let pools: &[(&str, &[&str])] = &[("COMPLEX", &["big"]), ("SIMPLE", &["mini"])];
    let facts = priced(&[("big", 2), ("mini", 1)]);
    let router =
        adaptive_service(adaptive_settings(true), pools, &facts, &[]).expect("the router builds");
    let snapshot = router.snapshot().expect("adaptive builds a snapshot");
    assert_eq!(snapshot.available_models, vec!["mini", "big"]);
}

#[test]
fn the_snapshot_orders_cells_by_request_type_name_then_model() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let router =
        adaptive_service(adaptive_settings(true), POOLS, &facts, &[]).expect("the router builds");
    let snapshot = router.snapshot().expect("adaptive builds a snapshot");
    let first = snapshot.cells.first().expect("the snapshot lists cells");
    assert_eq!(first.request_type, RequestType::AnalyticalReasoning);
    assert_eq!(first.model, "big");
    let last = snapshot.cells.last().expect("the snapshot lists cells");
    assert_eq!(last.request_type, RequestType::Writing);
    assert_eq!(last.model, "mini");
}
