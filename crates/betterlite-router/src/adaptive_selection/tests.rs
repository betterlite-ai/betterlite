#![cfg(test)]

use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::{Content, Message};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::costs::normalized_cost;
use super::error::AdaptiveSelectionError;
use super::pre_routing::{PreRoutingRequest, RouterKind, RoutingCause};
use super::{AdaptiveSelection, REQUEST_TYPES, default_preferences};
use crate::adaptive_classification::config::{
    AdaptiveRouterConfig, AdaptiveRouterPreferences, AdaptiveRouterWeights,
};
use crate::adaptive_classification::request_type::RequestType;
use crate::bandit_math::error::BanditMathError;
use crate::bandit_math::sampler::{QualitySampler, ScriptedSampler};

const fn weights(quality: f64, economy: f64) -> AdaptiveRouterWeights {
    AdaptiveRouterWeights { quality, economy }
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn message(role: &str, text: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Text(text.to_owned())),
        name: None,
        provider_specific_fields: IndexMap::new(),
    }
}

fn selection(
    models: &[&str],
    preferences: &[(&str, u8, &[RequestType])],
    costs: &[(&str, &str)],
    router_weights: AdaptiveRouterWeights,
    samples: &[f64],
) -> AdaptiveSelection {
    let mut preference_map = IndexMap::new();
    for (model, tier, strengths) in preferences {
        preference_map.insert(
            (*model).to_owned(),
            AdaptiveRouterPreferences {
                quality_tier: *tier,
                strengths: strengths.to_vec(),
            },
        );
    }
    let mut cost_map = IndexMap::new();
    for (model, amount) in costs {
        cost_map.insert((*model).to_owned(), decimal(amount));
    }
    AdaptiveSelection::new(
        "probe".to_owned(),
        &AdaptiveRouterConfig {
            available_models: models.iter().map(|model| (*model).to_owned()).collect(),
            weights: router_weights,
        },
        &preference_map,
        &cost_map,
        QualitySampler::Scripted(ScriptedSampler::new(samples.to_vec())),
    )
    .unwrap()
}

#[rstest]
#[case::balanced_prior_wins(
    weights(0.7, 0.3),
    &[0.2, 0.9, 0.1],
    &[("fast", "0.0"), ("balanced", "0.5"), ("top", "1.0")],
    None,
    "balanced"
)]
#[case::cost_weight_picks_the_cheapest(
    weights(0.0, 1.0),
    &[0.9, 0.1, 0.5],
    &[("fast", "1.0"), ("balanced", "0.0"), ("top", "0.5")],
    None,
    "balanced"
)]
#[case::ties_keep_the_first_model(
    weights(0.7, 0.3),
    &[0.5, 0.5, 0.5],
    &[("fast", "0.5"), ("balanced", "0.5"), ("top", "0.5")],
    None,
    "fast"
)]
#[case::tier_three_narrows_to_top(
    weights(0.7, 0.3),
    &[0.5],
    &[("fast", "0.0"), ("balanced", "0.5"), ("top", "1.0")],
    Some(3),
    "top"
)]
#[case::default_tier_keeps_two_models(
    weights(0.7, 0.3),
    &[0.9, 0.1],
    &[("fast", "0.0"), ("balanced", "0.5"), ("top", "1.0")],
    Some(2),
    "balanced"
)]
#[case::absent_cost_counts_as_free(
    weights(0.0, 1.0),
    &[0.5, 0.5, 0.5],
    &[("fast", "1.0")],
    None,
    "balanced"
)]
fn adaptive_selection_behaviors(
    #[case] router_weights: AdaptiveRouterWeights,
    #[case] samples: &[f64],
    #[case] costs: &[(&str, &str)],
    #[case] min_quality_tier: Option<i64>,
    #[case] expected: &str,
) {
    let preferences = [
        ("fast", 1, &[][..]),
        ("top", 3, &[RequestType::CodeGeneration][..]),
    ];
    let mut service = selection(
        &["fast", "balanced", "top"],
        &preferences,
        costs,
        router_weights,
        samples,
    );
    let selected = service
        .pick_model(RequestType::General, min_quality_tier)
        .unwrap();
    assert_eq!(selected, expected);
}

#[derive(Debug, PartialEq, Eq)]
enum BoundaryOutcome {
    Selected(&'static str),
    NoEligible(&'static str),
}

#[rstest]
#[case::no_minimum(None, BoundaryOutcome::Selected("fast"))]
#[case::negative_tier(Some(-1), BoundaryOutcome::Selected("fast"))]
#[case::tier_zero(Some(0), BoundaryOutcome::Selected("fast"))]
#[case::tier_one(Some(1), BoundaryOutcome::Selected("fast"))]
#[case::tier_two_keeps_the_default(Some(2), BoundaryOutcome::Selected("default"))]
#[case::tier_three_excludes_the_default(Some(3), BoundaryOutcome::NoEligible("3"))]
#[case::above_every_tier(Some(4), BoundaryOutcome::NoEligible("4"))]
fn adaptive_selection_boundaries(
    #[case] min_quality_tier: Option<i64>,
    #[case] expected: BoundaryOutcome,
) {
    let mut service = selection(
        &["fast", "default"],
        &[("fast", 1, &[][..])],
        &[("fast", "0.0"), ("default", "0.0")],
        weights(1.0, 0.0),
        &[0.9, 0.1],
    );
    match expected {
        BoundaryOutcome::Selected(model) => {
            let selected = service
                .pick_model(RequestType::General, min_quality_tier)
                .unwrap();
            assert_eq!(selected, model);
        }
        BoundaryOutcome::NoEligible(tier) => {
            let error = service
                .pick_model(RequestType::General, min_quality_tier)
                .unwrap_err();
            let message = error.to_string();
            match error {
                AdaptiveSelectionError::NoEligibleModels {
                    router_name,
                    tier: reported,
                } => {
                    assert_eq!(router_name, "probe");
                    assert_eq!(reported, tier);
                    assert_eq!(
                        message,
                        format!("AdaptiveRouter[probe]: no models meet min_quality_tier={tier}")
                    );
                }
                other => panic!("expected a refused tier, got {other:?}"),
            }
        }
    }
}

#[rstest]
#[case::no_spread("0.5", &["0.5", "0.5"], "0.500000")]
#[case::empty_costs("0.5", &[], "0.500000")]
#[case::single_cost("0.25", &["0.25"], "0.500000")]
#[case::cheapest("0.0", &["0.0", "1.0"], "1.000000")]
#[case::dearest("1.0", &["0.0", "1.0"], "0.000000")]
#[case::middle("0.25", &["0.0", "0.5", "1.0"], "0.750000")]
#[case::unsorted("0.5", &["1.0", "0.0"], "0.500000")]
#[case::above_the_range("2.0", &["0.0", "1.0"], "-1.000000")]
fn the_cost_score_maps_price_into_the_unit_range(
    #[case] cost: &str,
    #[case] all_costs: &[&str],
    #[case] expected: &str,
) {
    let amounts = all_costs
        .iter()
        .map(|text| decimal(text))
        .collect::<Vec<_>>();
    let cost_score = normalized_cost(decimal(cost), &amounts).unwrap();
    assert_eq!(format!("{cost_score:.6}"), expected);
}

#[test]
fn a_cost_spread_beyond_decimal_is_refused() {
    let highest = decimal("79228162514264337593543950335");
    let lowest = decimal("-79228162514264337593543950335");
    let error = normalized_cost(decimal("0"), &[highest, lowest]).unwrap_err();
    assert!(matches!(error, AdaptiveSelectionError::CostNormalization));
}

#[rstest]
#[case::created(None, r#"{"adaptive_router_chosen_model":"fast"}"#)]
#[case::kept(
    Some(r#"{"keep":1}"#),
    r#"{"keep":1,"adaptive_router_chosen_model":"fast"}"#
)]
#[case::replaced(
    Some(r#"{"adaptive_router_chosen_model":"old"}"#),
    r#"{"adaptive_router_chosen_model":"fast"}"#
)]
#[case::null_is_kept(Some("null"), "null")]
#[case::scalar_is_kept(Some("7"), "7")]
#[case::list_is_kept(Some("[1]"), "[1]")]
fn the_stash_only_lands_in_a_metadata_object(
    #[case] metadata: Option<&str>,
    #[case] expected: &str,
) {
    let mut service = selection(&["fast"], &[], &[], weights(0.7, 0.3), &[0.5]);
    let mut stored = metadata.map(json);
    let messages = vec![message("user", "Hello.")];
    let response = service
        .pre_route(PreRoutingRequest {
            messages: Some(&messages),
            headers: None,
            metadata: &mut stored,
        })
        .unwrap();
    assert_eq!(response.model, "fast");
    assert_eq!(stored.unwrap(), json(expected));
}

#[test]
fn the_pre_routing_hook_classifies_and_reports_the_decision() {
    let mut service = selection(&["fast"], &[], &[], weights(0.7, 0.3), &[0.5]);
    let messages = vec![message(
        "user",
        "Write a Python function that reverses a list.",
    )];
    let mut stored = None;
    let response = service
        .pre_route(PreRoutingRequest {
            messages: Some(&messages),
            headers: None,
            metadata: &mut stored,
        })
        .unwrap();
    assert_eq!(response.model, "fast");
    assert_eq!(response.messages.unwrap().len(), 1);
    assert_eq!(response.routing_decision.router_model_name, "probe");
    assert_eq!(response.routing_decision.router_type, RouterKind::Adaptive);
    assert_eq!(response.routing_decision.routed_model, "fast");
    assert_eq!(response.routing_decision.cause, RoutingCause::Bandit);
    assert_eq!(
        response.routing_decision.request_type,
        RequestType::CodeGeneration
    );
}

#[test]
fn a_request_without_messages_is_general() {
    let mut service = selection(&["fast"], &[], &[], weights(0.7, 0.3), &[0.5]);
    let mut stored = None;
    let response = service
        .pre_route(PreRoutingRequest {
            messages: None,
            headers: None,
            metadata: &mut stored,
        })
        .unwrap();
    assert_eq!(response.routing_decision.request_type, RequestType::General);
    assert!(response.messages.is_none());
}

#[rstest]
#[case::header(Some(r#"{"x-litellm-min-quality-tier":"3"}"#), None, "top")]
#[case::metadata(None, Some(r#"{"min_quality_tier":3}"#), "top")]
#[case::header_wins(
    Some(r#"{"x-litellm-min-quality-tier":"1"}"#),
    Some(r#"{"min_quality_tier":3}"#),
    "fast"
)]
#[case::broken_header_blocks_metadata(
    Some(r#"{"x-litellm-min-quality-tier":"many"}"#),
    Some(r#"{"min_quality_tier":3}"#),
    "fast"
)]
fn the_hook_filters_by_the_requested_tier(
    #[case] headers: Option<&str>,
    #[case] metadata: Option<&str>,
    #[case] expected: &str,
) {
    let mut service = selection(
        &["fast", "top"],
        &[("fast", 1, &[][..]), ("top", 3, &[][..])],
        &[("fast", "0.0"), ("top", "0.0")],
        weights(1.0, 0.0),
        &[0.9, 0.1],
    );
    let resolved_headers = headers.map(json);
    let mut stored = metadata.map(json);
    let messages = vec![message("user", "Hello.")];
    let response = service
        .pre_route(PreRoutingRequest {
            messages: Some(&messages),
            headers: resolved_headers.as_ref(),
            metadata: &mut stored,
        })
        .unwrap();
    assert_eq!(response.model, expected);
}

#[test]
fn the_snapshot_lists_sorted_cells_with_their_priors() {
    let service = selection(
        &["b", "a"],
        &[("a", 3, &[RequestType::Writing][..]), ("b", 1, &[][..])],
        &[("b", "0.25"), ("a", "1.0")],
        weights(0.7, 0.3),
        &[],
    );
    let snapshot = service.state_snapshot();
    assert_eq!(snapshot.router_name, "probe");
    assert_eq!(
        snapshot.available_models,
        vec!["b".to_owned(), "a".to_owned()]
    );
    assert_eq!(snapshot.weights.quality.to_bits(), 0.7_f64.to_bits());
    assert_eq!(snapshot.weights.economy.to_bits(), 0.3_f64.to_bits());
    assert_eq!(
        snapshot.model_costs,
        vec![
            ("b".to_owned(), decimal("0.25")),
            ("a".to_owned(), decimal("1.0")),
        ]
    );
    assert_eq!(snapshot.cells.len(), 2 * REQUEST_TYPES.len());
    let types = snapshot
        .cells
        .iter()
        .step_by(2)
        .map(|cell| cell.request_type)
        .collect::<Vec<_>>();
    assert_eq!(
        types,
        vec![
            RequestType::AnalyticalReasoning,
            RequestType::CodeGeneration,
            RequestType::CodeUnderstanding,
            RequestType::FactualLookup,
            RequestType::General,
            RequestType::TechnicalDesign,
            RequestType::Writing,
        ]
    );
    let first = &snapshot.cells[0];
    assert_eq!(first.model, "a");
    assert_eq!(format!("{:.6}", first.alpha), "7.000000");
    assert_eq!(format!("{:.6}", first.beta), "3.000000");
    assert_eq!(first.samples, 0);
    assert_eq!(format!("{:.6}", first.quality_mean), "0.700000");
    let last = &snapshot.cells[snapshot.cells.len() - 1];
    assert_eq!(last.request_type, RequestType::Writing);
    assert_eq!(last.model, "b");
    assert_eq!(format!("{:.6}", last.alpha), "3.000000");
    assert_eq!(format!("{:.6}", last.beta), "7.000000");
}

#[test]
fn a_model_without_preferences_starts_at_tier_two() {
    let defaults = default_preferences();
    assert_eq!(defaults.quality_tier, 2);
    assert!(defaults.strengths.is_empty());
    let service = selection(&["solo"], &[], &[], weights(0.7, 0.3), &[]);
    let snapshot = service.state_snapshot();
    assert_eq!(snapshot.cells.len(), REQUEST_TYPES.len());
    assert!(snapshot.cells.iter().all(|cell| {
        format!("{:.6}", cell.alpha) == "5.000000" && format!("{:.6}", cell.beta) == "5.000000"
    }));
}

#[test]
fn an_empty_router_reports_the_missing_tier_as_none() {
    let mut service = selection(&[], &[], &[], weights(0.7, 0.3), &[]);
    let error = service.pick_model(RequestType::General, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "AdaptiveRouter[probe]: no models meet min_quality_tier=None"
    );
}

#[test]
fn an_unsupported_preference_tier_is_refused() {
    let mut preferences = IndexMap::new();
    preferences.insert(
        "fast".to_owned(),
        AdaptiveRouterPreferences {
            quality_tier: 4,
            strengths: Vec::new(),
        },
    );
    let config = AdaptiveRouterConfig {
        available_models: vec!["fast".to_owned()],
        weights: weights(0.7, 0.3),
    };
    let empty = IndexMap::new();
    let error = AdaptiveSelection::new(
        "probe".to_owned(),
        &config,
        &preferences,
        &empty,
        QualitySampler::Scripted(ScriptedSampler::new(Vec::new())),
    )
    .unwrap_err();
    match error {
        AdaptiveSelectionError::Bandit(BanditMathError::UnsupportedTier { tier }) => {
            assert_eq!(tier, 4);
        }
        other => panic!("expected an unsupported tier, got {other:?}"),
    }
}

#[test]
fn a_missing_scripted_sample_is_reported() {
    let mut service = selection(&["fast"], &[], &[], weights(0.7, 0.3), &[]);
    let error = service.pick_model(RequestType::General, None).unwrap_err();
    assert!(matches!(
        error,
        AdaptiveSelectionError::Bandit(BanditMathError::SamplesExhausted)
    ));
}

#[test]
fn a_non_finite_scripted_sample_is_refused() {
    let mut service = selection(&["fast"], &[], &[], weights(0.7, 0.3), &[f64::NAN]);
    let error = service.pick_model(RequestType::General, None).unwrap_err();
    match error {
        AdaptiveSelectionError::Bandit(BanditMathError::NonFinite { field }) => {
            assert_eq!(field, "scripted sample");
        }
        other => panic!("expected a refused sample, got {other:?}"),
    }
}
