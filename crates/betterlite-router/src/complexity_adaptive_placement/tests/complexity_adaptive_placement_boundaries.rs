use betterlite_config::sections::complexity_adaptive_placement::AdaptiveEligible;
use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::adaptive_classification::config::AdaptiveRouterWeights;
use crate::adaptive_classification::request_type::RequestType;
use crate::bandit_math::error::BanditMathError;
use crate::complexity_adaptive_placement::candidates::CandidateQuery;
use crate::complexity_adaptive_placement::decision::{
    AdaptiveDecision, ColdStartCandidate, ColdStartDecision,
};
use crate::complexity_adaptive_placement::error::AdaptivePlacementError;
use crate::complexity_adaptive_placement::pick::Placed;
use crate::soft_floor_math::placement_score;

use super::support::{
    Harness, POOLS, adaptive_service, adaptive_settings, cell_in, configured_settings, priced,
    request, service_error, tier, warm,
};

#[test]
fn a_non_adaptive_router_builds_without_pools() {
    let router = adaptive_service(adaptive_settings(false), &[], &priced(&[]), &[])
        .expect("adaptive off needs no pool");
    assert_eq!(router.snapshot(), None);
}

#[test]
fn an_adaptive_router_without_pools_is_rejected() {
    let error = service_error(adaptive_service(
        adaptive_settings(true),
        &[],
        &priced(&[]),
        &[],
    ));
    assert!(matches!(error, AdaptivePlacementError::NoTierPools));
}

#[test]
fn an_empty_configured_pool_is_rejected() {
    let empty: &[&str] = &[];
    let pools: &[(&str, &[&str])] = &[("SIMPLE", empty), ("MEDIUM", &["mid"])];
    let error = service_error(adaptive_service(
        adaptive_settings(true),
        pools,
        &priced(&[("mid", 1)]),
        &[],
    ));
    assert!(matches!(
        error,
        AdaptivePlacementError::EmptyTierPool { ref tier } if tier == "SIMPLE"
    ));
}

#[test]
fn a_router_whose_only_pool_is_empty_is_rejected() {
    let empty: &[&str] = &[];
    let pools: &[(&str, &[&str])] = &[("SIMPLE", empty)];
    let error = service_error(adaptive_service(
        adaptive_settings(true),
        pools,
        &priced(&[]),
        &[],
    ));
    assert!(matches!(error, AdaptivePlacementError::NoTierPools));
}

#[test]
fn observing_with_adaptive_off_is_rejected() {
    let mut router = adaptive_service(
        adaptive_settings(false),
        POOLS,
        &priced(&[("mini", 1)]),
        &[],
    )
    .expect("the router builds");
    let error = router
        .observe(RequestType::General, "mini", 1.0, 0.0)
        .expect_err("adaptive off has no cells");
    assert!(matches!(error, AdaptivePlacementError::AdaptiveDisabled));
}

#[test]
fn observing_an_unknown_model_names_it() {
    let mut router = adaptive_service(adaptive_settings(true), POOLS, &priced(&[("mini", 1)]), &[])
        .expect("the router builds");
    let error = router
        .observe(RequestType::General, "ghost", 1.0, 0.0)
        .expect_err("ghost has no cell");
    assert!(matches!(
        error,
        AdaptivePlacementError::MissingCell {
            request_type: RequestType::General,
            ref model,
        } if model == "ghost"
    ));
}

#[test]
fn a_nonfinite_observation_is_rejected() {
    let mut router = adaptive_service(adaptive_settings(true), POOLS, &priced(&[("mini", 1)]), &[])
        .expect("the router builds");
    let error = router
        .observe(RequestType::General, "mini", f64::NAN, 0.0)
        .expect_err("a NaN delta has no cell");
    assert!(matches!(
        error,
        AdaptivePlacementError::Bandit(BanditMathError::NonFinite { field: "alpha" })
    ));
}

#[test]
fn a_floor_tier_outside_the_router_is_named() {
    let router = adaptive_service(adaptive_settings(true), POOLS, &priced(&[("mini", 1)]), &[])
        .expect("the router builds");
    let classified = tier("MEDIUM");
    let floor = tier("legacy");
    let query = CandidateQuery {
        classified_tier: &classified,
        hard_floor: Some(&floor),
        hard_ceiling: None,
        fit_filter: None,
    };
    let error = router
        .candidate_models(&query)
        .expect_err("legacy has no severity");
    assert!(matches!(
        error,
        AdaptivePlacementError::UnknownTier { ref tier } if tier == "legacy"
    ));
}

#[test]
fn a_nonfinite_scripted_sample_is_rejected() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(adaptive_settings(true), POOLS, &facts, &[f64::NAN], None)
        .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let error = harness
        .pick(&request(&classified, "hello"))
        .expect_err("a NaN sample has no score");
    assert!(matches!(
        error,
        AdaptivePlacementError::Bandit(BanditMathError::NonFinite {
            field: "scripted sample"
        })
    ));
}

#[test]
fn a_script_that_runs_out_is_reported() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(adaptive_settings(true), POOLS, &facts, &[0.5], None)
        .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid"]);
    let classified = tier("MEDIUM");
    let error = harness
        .pick(&request(&classified, "hello"))
        .expect_err("three candidates need three samples");
    assert!(matches!(
        error,
        AdaptivePlacementError::Bandit(BanditMathError::SamplesExhausted)
    ));
}

#[test]
fn a_tie_keeps_the_first_candidate() {
    let pools: &[(&str, &[&str])] = &[
        ("SIMPLE", &["mini"]),
        ("MEDIUM", &["mid", "mid2"]),
        ("COMPLEX", &["big"]),
    ];
    let facts = priced(&[("mini", 1), ("mid", 1), ("mid2", 1), ("big", 1)]);
    let mut harness = Harness::new(
        adaptive_settings(true),
        pools,
        &facts,
        &[0.1, 0.5, 0.5, 0.1],
        None,
    )
    .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mid", "mid2"]);
    let classified = tier("MEDIUM");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mid");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    let models: Vec<&str> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.model.as_str())
        .collect();
    assert_eq!(models, vec!["mini", "mid", "mid2", "big"]);
    let scores: Vec<u64> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.score.to_bits())
        .collect();
    assert_eq!(scores[1], scores[2]);
}

// the distance is the minimum over every pool the model sits in, not its first membership
#[test]
fn a_multi_pool_candidate_takes_its_nearest_membership_distance() {
    let pools: &[(&str, &[&str])] = &[
        ("SIMPLE", &["mini"]),
        ("MEDIUM", &["mid"]),
        ("COMPLEX", &["mini"]),
    ];
    // equal prices leave only the distance to separate the two candidates
    let facts = priced(&[("mini", 1), ("mid", 1)]);
    let mut harness = Harness::new(adaptive_settings(true), pools, &facts, &[0.9, 0.9], None)
        .expect("the router builds");
    warm(&mut harness.router, RequestType::General, &["mini"]);
    let classified = tier("SIMPLE");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mini");
    let Some(AdaptiveDecision::Adaptive(choice)) = &placed.decision else {
        panic!("the warm pool takes the adaptive phase");
    };
    let models: Vec<&str> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.model.as_str())
        .collect();
    assert_eq!(models, vec!["mini", "mid"]);
    let distances: Vec<u32> = choice
        .candidates
        .iter()
        .map(|candidate| candidate.tier_distance)
        .collect();
    assert_eq!(distances, vec![0, 1]);
}

// cold start draws only the classified pool's unobserved models, never the warm ones beside them
#[rstest]
fn a_partly_cold_pool_draws_only_its_unobserved_models(
    #[values(0_u64, 1_u64, 2_u64, 3_u64, 4_u64, 5_u64, 6_u64, 7_u64)] seed: u64,
) {
    let pools: &[(&str, &[&str])] = &[
        ("SIMPLE", &["mini", "small"]),
        ("MEDIUM", &["mid"]),
        ("COMPLEX", &["big"]),
    ];
    let facts = priced(&[("mini", 1), ("small", 1), ("mid", 2), ("big", 3)]);
    let mut harness =
        Harness::new(adaptive_settings(true), pools, &facts, &[], None).expect("the router builds");
    harness.reseed(seed);
    warm(&mut harness.router, RequestType::General, &["mini"]);
    let classified = tier("SIMPLE");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(
        placed,
        Placed {
            model: "small".to_owned(),
            decision: Some(AdaptiveDecision::ColdStart(ColdStartDecision {
                classified_tier: "SIMPLE".to_owned(),
                request_type: RequestType::General,
                weights: AdaptiveRouterWeights {
                    quality: 0.3,
                    economy: 0.7,
                },
                tier_distance_penalty: 0.5,
                chosen_model: "small".to_owned(),
                candidates: vec![ColdStartCandidate {
                    model: "small".to_owned(),
                    total_samples: 0,
                }],
            })),
        }
    );
}

#[rstest]
#[case::penalty_zero(
    0.5_f64,
    0.5_f64,
    AdaptiveRouterWeights { quality: 0.5, economy: 0.5 },
    0_usize,
    0.5_f64
)]
#[case::one_tier_away(
    0.5_f64,
    0.5_f64,
    AdaptiveRouterWeights { quality: 0.5, economy: 0.5 },
    1_usize,
    0.0_f64
)]
#[case::two_tiers_away(
    0.5_f64,
    0.5_f64,
    AdaptiveRouterWeights { quality: 0.5, economy: 0.5 },
    2_usize,
    -0.5_f64
)]
#[case::quality_only(
    1.0_f64,
    0.0_f64,
    AdaptiveRouterWeights { quality: 1.0, economy: 0.0 },
    0_usize,
    1.0_f64
)]
#[case::cost_only(
    0.0_f64,
    1.0_f64,
    AdaptiveRouterWeights { quality: 0.0, economy: 0.5 },
    0_usize,
    0.5_f64
)]
fn the_score_is_the_weighted_sum_minus_the_distance_penalty(
    #[case] quality_sample: f64,
    #[case] cost_score: f64,
    #[case] weights: AdaptiveRouterWeights,
    #[case] distance: usize,
    #[case] expected: f64,
) {
    let score = placement_score(weights, quality_sample, cost_score, 0.5, distance)
        .expect("finite inputs score");
    assert_eq!(score.to_bits(), expected.to_bits());
}

#[test]
fn a_nonfinite_quality_sample_is_rejected() {
    let error = placement_score(
        AdaptiveRouterWeights {
            quality: 0.3,
            economy: 0.7,
        },
        f64::NAN,
        0.5,
        0.5,
        0,
    )
    .expect_err("NaN is not a sample");
    assert!(matches!(
        error,
        BanditMathError::NonFinite {
            field: "quality sample"
        }
    ));
}

#[test]
fn a_nonfinite_cost_score_is_rejected() {
    let error = placement_score(
        AdaptiveRouterWeights {
            quality: 0.3,
            economy: 0.7,
        },
        0.5,
        f64::INFINITY,
        0.5,
        0,
    )
    .expect_err("an infinite cost score has no score");
    assert!(matches!(
        error,
        BanditMathError::NonFinite {
            field: "cost score"
        }
    ));
}

#[test]
fn a_nonfinite_weight_is_rejected() {
    let error = placement_score(
        AdaptiveRouterWeights {
            quality: f64::NAN,
            economy: 0.7,
        },
        0.5,
        0.5,
        0.5,
        0,
    )
    .expect_err("a NaN weight has no score");
    assert!(matches!(
        error,
        BanditMathError::NonFinite {
            field: "score weight"
        }
    ));
    let infinite_error = placement_score(
        AdaptiveRouterWeights {
            quality: 0.3,
            economy: f64::NEG_INFINITY,
        },
        0.5,
        0.5,
        0.5,
        0,
    )
    .expect_err("an infinite weight has no score");
    assert!(matches!(
        infinite_error,
        BanditMathError::NonFinite {
            field: "score weight"
        }
    ));
}

#[test]
fn a_nonfinite_penalty_is_rejected() {
    let error = placement_score(
        AdaptiveRouterWeights {
            quality: 0.3,
            economy: 0.7,
        },
        0.5,
        0.5,
        f64::NAN,
        0,
    )
    .expect_err("a NaN penalty has no score");
    assert!(matches!(
        error,
        BanditMathError::NonFinite {
            field: "tier distance penalty"
        }
    ));
}

#[test]
fn a_score_that_overflows_is_rejected() {
    let error = placement_score(
        AdaptiveRouterWeights {
            quality: f64::MAX,
            economy: 0.0,
        },
        2.0,
        0.0,
        0.5,
        0,
    )
    .expect_err("the sum overflows to infinity");
    assert!(matches!(
        error,
        BanditMathError::NonFinite {
            field: "selection score"
        }
    ));
}

#[test]
fn an_update_past_the_sample_cap_leaves_the_cell() {
    let mut router = adaptive_service(adaptive_settings(true), POOLS, &priced(&[("mini", 1)]), &[])
        .expect("the router builds");
    router
        .observe(RequestType::General, "mini", 190.0, 0.0)
        .expect("an update up to the cap is accepted");
    let snapshot = router.snapshot().expect("adaptive builds a snapshot");
    let cell = cell_in(&snapshot, RequestType::General, "mini");
    assert_eq!(cell.alpha.to_bits(), 195.0_f64.to_bits());
    assert_eq!(cell.samples, 190);
    router
        .observe(RequestType::General, "mini", 1.0, 0.0)
        .expect("the update past the cap is dropped");
    let capped_snapshot = router.snapshot().expect("adaptive builds a snapshot");
    let capped_cell = cell_in(&capped_snapshot, RequestType::General, "mini");
    assert_eq!(capped_cell.alpha.to_bits(), 195.0_f64.to_bits());
    assert_eq!(capped_cell.samples, 190);
}

#[test]
fn a_classified_tier_without_a_pool_falls_back() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness = Harness::new(
        configured_settings(true, 0.5, AdaptiveEligible::ClassifiedTier),
        POOLS,
        &facts,
        &[],
        None,
    )
    .expect("the router builds");
    let classified = tier("REASONING");
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
}

#[test]
fn a_non_object_metadata_is_left_as_it_was() {
    let facts = priced(&[("mini", 1), ("mid", 2), ("big", 3)]);
    let mut harness =
        Harness::new(adaptive_settings(true), POOLS, &facts, &[], None).expect("the router builds");
    harness
        .kwargs
        .insert("metadata".to_owned(), Json::Text("keep".to_owned()));
    let classified = tier("SIMPLE");
    let placed = harness
        .pick(&request(&classified, "hello"))
        .expect("the pick succeeds");
    assert_eq!(placed.model, "mini");
    assert!(matches!(
        harness.kwargs.get("metadata"),
        Some(Json::Text(text)) if text == "keep"
    ));
}
