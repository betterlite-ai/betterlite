#![cfg(test)]

use rstest::rstest;

use super::classifier::Classifier;
use super::config::{AdaptiveRouterWeights, resolve};
use super::constants::{
    ADAPTIVE_MODEL_HEADER, BASE_TIER_WEIGHT, CHOSEN_MODEL_KEY, COLD_START_MASS,
    DEFAULT_COST_WEIGHT, DEFAULT_QUALITY_WEIGHT, LOOP_REPEAT_THRESHOLD, MIN_QUALITY_TIER_HEADER,
    MIN_QUALITY_TIER_METADATA_KEY, MIN_TURNS_FOR_CLEAN_CREDIT, MISALIGNMENT_JACCARD_THRESHOLD,
    OWNER_CACHE_TTL_SECONDS, SAMPLE_CAP, SIGNAL_GATE_MIN_MESSAGES, STAGNATION_JACCARD_NEAR_DUP,
    STRENGTH_BONUS, TOOL_CALL_HISTORY_MAX,
};
use super::error::AdaptiveClassificationError;
use super::minimum_quality::minimum_quality_tier;
use super::request_type::RequestType;
use crate::preference_math;
use betterlite_config::sections::adaptive_classification as section;
use betterlite_models::decisions::Json;

fn classify(text: &str) -> RequestType {
    Classifier::new().unwrap().classify_prompt(text)
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::language_code_generation(
    "Write a Python function that reverses a list.",
    RequestType::CodeGeneration
)]
#[case::artefact_code_generation(
    "Create a helper function to sort numbers.",
    RequestType::CodeGeneration
)]
#[case::explanation_code_understanding(
    "Explain what this function does.",
    RequestType::CodeUnderstanding
)]
#[case::error_code_understanding("Debug this error in my code.", RequestType::CodeUnderstanding)]
#[case::review_code_understanding(
    "Review this pull request for style.",
    RequestType::CodeUnderstanding
)]
#[case::system_technical_design(
    "Design a database schema for invoices.",
    RequestType::TechnicalDesign
)]
#[case::choice_technical_design(
    "Should I use Postgres or MySQL for this service?",
    RequestType::TechnicalDesign
)]
#[case::shape_technical_design("How should I structure my module?", RequestType::TechnicalDesign)]
#[case::equation_analytical_reasoning(
    "Solve this integral for me.",
    RequestType::AnalyticalReasoning
)]
#[case::suppose_analytical_reasoning(
    "Suppose we have two coins.",
    RequestType::AnalyticalReasoning
)]
#[case::probability_analytical_reasoning(
    "What is the probability of rain tomorrow?",
    RequestType::AnalyticalReasoning
)]
#[case::email_writing("Write an email to my manager.", RequestType::Writing)]
#[case::concise_writing("Make this more concise.", RequestType::Writing)]
#[case::president_factual_lookup(
    "Who is the current president of France?",
    RequestType::FactualLookup
)]
#[case::definition_factual_lookup("Define photosynthesis.", RequestType::FactualLookup)]
#[case::spelling_factual_lookup(
    "How many days are there in a leap year?",
    RequestType::FactualLookup
)]
#[case::unmatched_general("Tell me a joke about penguins.", RequestType::General)]
#[case::empty_general("", RequestType::General)]
#[case::whitespace_general("  \n\t ", RequestType::General)]
fn adaptive_classification_behaviors(#[case] prompt: &str, #[case] expected: RequestType) {
    assert_eq!(classify(prompt), expected);
}

#[derive(Debug, PartialEq)]
enum WeightsOutcome {
    Accepted(f64, f64),
    QualityWeight,
    EconomyWeight,
    Sum(f64, f64),
}

#[rstest]
#[case::defaults(None, None, WeightsOutcome::Accepted(0.7, 0.3))]
#[case::all_quality(Some(1.0), Some(0.0), WeightsOutcome::Accepted(1.0, 0.0))]
#[case::all_economy(Some(0.0), Some(1.0), WeightsOutcome::Accepted(0.0, 1.0))]
#[case::inside_tolerance(Some(0.75), Some(0.251), WeightsOutcome::Accepted(0.75, 0.251))]
#[case::outside_tolerance(Some(0.75), Some(0.249), WeightsOutcome::Sum(0.75, 0.249))]
#[case::just_inside_tolerance(Some(0.7009), Some(0.3), WeightsOutcome::Accepted(0.7009, 0.3))]
#[case::just_outside_tolerance(Some(0.7012), Some(0.3), WeightsOutcome::Sum(0.7012, 0.3))]
#[case::far_from_one(Some(0.9), Some(0.5), WeightsOutcome::Sum(0.9, 0.5))]
#[case::quality_alone(Some(0.9), None, WeightsOutcome::Accepted(0.9, 0.3))]
#[case::cost_alone_matching(None, Some(0.3), WeightsOutcome::Accepted(0.7, 0.3))]
#[case::cost_alone_off_sum(None, Some(0.4), WeightsOutcome::Sum(0.7, 0.4))]
#[case::quality_below_zero(Some(-0.1), Some(1.1), WeightsOutcome::QualityWeight)]
#[case::economy_above_one(Some(0.5), Some(1.5), WeightsOutcome::EconomyWeight)]
#[case::economy_below_zero(Some(0.5), Some(-0.2), WeightsOutcome::EconomyWeight)]
#[case::not_a_number(Some(f64::NAN), None, WeightsOutcome::QualityWeight)]
#[case::not_a_number_cost(Some(0.7), Some(f64::NAN), WeightsOutcome::EconomyWeight)]
fn adaptive_classification_boundaries(
    #[case] quality: Option<f64>,
    #[case] cost: Option<f64>,
    #[case] expected: WeightsOutcome,
) {
    let outcome = AdaptiveRouterWeights::resolve(Some(&section::Weights {
        quality,
        economy: cost,
    }));
    match expected {
        WeightsOutcome::Accepted(expected_quality, expected_economy) => {
            let weights = outcome.unwrap();
            assert_eq!(weights.quality.to_bits(), expected_quality.to_bits());
            assert_eq!(weights.economy.to_bits(), expected_economy.to_bits());
        }
        WeightsOutcome::QualityWeight => match outcome {
            Err(AdaptiveClassificationError::QualityWeight { value }) => {
                assert_eq!(value.to_bits(), quality.unwrap().to_bits());
            }
            other => panic!("expected an out-of-range quality weight, got {other:?}"),
        },
        WeightsOutcome::EconomyWeight => match outcome {
            Err(AdaptiveClassificationError::EconomyWeight { value }) => {
                assert_eq!(value.to_bits(), cost.unwrap().to_bits());
            }
            other => panic!("expected an out-of-range cost weight, got {other:?}"),
        },
        WeightsOutcome::Sum(expected_quality, expected_economy) => match outcome {
            Err(AdaptiveClassificationError::WeightsSum {
                quality: reported_quality,
                economy: reported_economy,
                sum,
            }) => {
                assert_eq!(reported_quality.to_bits(), expected_quality.to_bits());
                assert_eq!(reported_economy.to_bits(), expected_economy.to_bits());
                let total = preference_math::weights_total(reported_quality, reported_economy);
                assert_eq!(sum.to_bits(), total.total.to_bits());
            }
            other => panic!("expected weights off their sum, got {other:?}"),
        },
    }
}

#[derive(Debug, Eq, PartialEq)]
enum PreferencesOutcome {
    Accepted(u8, Vec<RequestType>),
    Tier,
    Strength,
}

#[rstest]
#[case::lowest_tier(1, &[], PreferencesOutcome::Accepted(1, vec![]))]
#[case::middle_tier(2, &["code_generation"], PreferencesOutcome::Accepted(2, vec![RequestType::CodeGeneration]))]
#[case::highest_tier(3, &["writing", "general"], PreferencesOutcome::Accepted(3, vec![RequestType::Writing, RequestType::General]))]
#[case::tier_zero(0, &[], PreferencesOutcome::Tier)]
#[case::tier_four(4, &[], PreferencesOutcome::Tier)]
#[case::tier_negative(-1, &[], PreferencesOutcome::Tier)]
#[case::unknown_strength(2, &["nope"], PreferencesOutcome::Strength)]
#[case::uppercase_strength(2, &["CODE_GENERATION"], PreferencesOutcome::Strength)]
fn preferences_keep_their_declared_bounds(
    #[case] quality_tier: i64,
    #[case] strengths: &[&str],
    #[case] expected: PreferencesOutcome,
) {
    let input = section::Preferences {
        quality_tier,
        strengths: strengths.iter().map(|name| (*name).to_owned()).collect(),
    };
    let outcome = super::config::AdaptiveRouterPreferences::resolve(&input);
    match expected {
        PreferencesOutcome::Accepted(expected_tier, expected_strengths) => {
            let resolved = outcome.unwrap();
            assert_eq!(resolved.quality_tier, expected_tier);
            assert_eq!(resolved.strengths, expected_strengths);
        }
        PreferencesOutcome::Tier => match outcome {
            Err(AdaptiveClassificationError::QualityTier { value }) => {
                assert_eq!(value, quality_tier);
            }
            other => panic!("expected an out-of-range quality tier, got {other:?}"),
        },
        PreferencesOutcome::Strength => match outcome {
            Err(AdaptiveClassificationError::UnknownStrength { value }) => {
                assert_eq!(value, strengths[0]);
            }
            other => panic!("expected an unknown strength, got {other:?}"),
        },
    }
}

#[test]
fn classifier_keeps_the_first_two_thousand_characters() {
    let exact = format!("{} write python", "a".repeat(1987));
    assert_eq!(classify(&exact), RequestType::CodeGeneration);
    let over = format!("{} write python", "a".repeat(1988));
    assert_eq!(classify(&over), RequestType::General);
    let multibyte = format!("{} write python", "é".repeat(1985));
    assert_eq!(classify(&multibyte), RequestType::CodeGeneration);
    let past_boundary = format!("{} write python", "é".repeat(1990));
    assert_eq!(classify(&past_boundary), RequestType::General);
}

#[test]
fn constants_keep_the_pinned_values() {
    assert_eq!(DEFAULT_QUALITY_WEIGHT.to_bits(), 0.7_f64.to_bits());
    assert_eq!(DEFAULT_COST_WEIGHT.to_bits(), 0.3_f64.to_bits());
    assert_eq!(BASE_TIER_WEIGHT.map(|entry| entry.0), [1, 2, 3]);
    assert_eq!(
        BASE_TIER_WEIGHT.map(|entry| entry.1.to_bits()),
        [0.3_f64.to_bits(), 0.5_f64.to_bits(), 0.7_f64.to_bits()]
    );
    assert_eq!(STRENGTH_BONUS.to_bits(), 0.3_f64.to_bits());
    assert_eq!(COLD_START_MASS.to_bits(), 10.0_f64.to_bits());
    assert_eq!(SAMPLE_CAP, 200);
    assert_eq!(MIN_TURNS_FOR_CLEAN_CREDIT, 3);
    assert_eq!(OWNER_CACHE_TTL_SECONDS, 86_400);
    assert_eq!(SIGNAL_GATE_MIN_MESSAGES, 4);
    assert_eq!(MISALIGNMENT_JACCARD_THRESHOLD.to_bits(), 0.45_f64.to_bits());
    assert_eq!(STAGNATION_JACCARD_NEAR_DUP.to_bits(), 0.5_f64.to_bits());
    assert_eq!(LOOP_REPEAT_THRESHOLD, 3);
    assert_eq!(TOOL_CALL_HISTORY_MAX, 20);
    assert_eq!(MIN_QUALITY_TIER_HEADER, "x-litellm-min-quality-tier");
    assert_eq!(MIN_QUALITY_TIER_METADATA_KEY, "min_quality_tier");
    assert_eq!(CHOSEN_MODEL_KEY, "adaptive_router_chosen_model");
    assert_eq!(ADAPTIVE_MODEL_HEADER, "x-litellm-adaptive-router-model");
}

#[rstest]
#[case::lowercase_header(Some(r#"{"x-litellm-min-quality-tier":"3"}"#), None, Some(3))]
#[case::mixed_case_header(Some(r#"{"X-LiteLLM-Min-Quality-Tier":"2"}"#), None, Some(2))]
#[case::padded_header_value(Some(r#"{"x-litellm-min-quality-tier":" 2 "}"#), None, Some(2))]
#[case::numeric_header(Some(r#"{"x-litellm-min-quality-tier":2}"#), None, Some(2))]
#[case::negative_header(Some(r#"{"x-litellm-min-quality-tier":"-1"}"#), None, Some(-1))]
#[case::metadata_number(None, Some(r#"{"min_quality_tier":3}"#), Some(3))]
#[case::metadata_text(None, Some(r#"{"min_quality_tier":"3"}"#), Some(3))]
#[case::header_wins(
    Some(r#"{"x-litellm-min-quality-tier":"3"}"#),
    Some(r#"{"min_quality_tier":1}"#),
    Some(3)
)]
#[case::broken_header_blocks_metadata(
    Some(r#"{"x-litellm-min-quality-tier":"many"}"#),
    Some(r#"{"min_quality_tier":2}"#),
    None
)]
#[case::other_header_falls_through(
    Some(r#"{"x-other-tier":"3"}"#),
    Some(r#"{"min_quality_tier":1}"#),
    Some(1)
)]
#[case::headers_not_an_object(Some(r#"["3"]"#), Some(r#"{"min_quality_tier":1}"#), Some(1))]
#[case::absent(None, None, None)]
#[case::null_metadata(None, Some("null"), None)]
#[case::metadata_other_key(None, Some(r#"{"other":3}"#), None)]
#[case::metadata_null_field(None, Some(r#"{"min_quality_tier":null}"#), None)]
#[case::metadata_fraction(None, Some(r#"{"min_quality_tier":2.7}"#), Some(2))]
#[case::metadata_text_fraction(None, Some(r#"{"min_quality_tier":"2.7"}"#), None)]
#[case::metadata_bool(None, Some(r#"{"min_quality_tier":true}"#), None)]
#[case::metadata_huge_float(None, Some(r#"{"min_quality_tier":1e20}"#), Some(i64::MAX))]
#[case::metadata_huge_negative(None, Some(r#"{"min_quality_tier":-1e20}"#), Some(i64::MIN))]
fn minimum_quality_tier_reads_headers_before_metadata(
    #[case] headers: Option<&str>,
    #[case] metadata: Option<&str>,
    #[case] expected: Option<i64>,
) {
    let resolved_headers = headers.map(json);
    let resolved_metadata = metadata.map(json);
    assert_eq!(
        minimum_quality_tier(resolved_headers.as_ref(), resolved_metadata.as_ref()),
        expected
    );
}

#[test]
fn resolve_keeps_settings_aligned_with_the_model_list() {
    let settings = section::Settings {
        deployments: vec![
            Some(section::Deployment {
                model_name: "router-a".to_owned(),
                config: Some(section::RouterConfig {
                    available_models: vec!["fast".to_owned()],
                    weights: None,
                }),
                preferences: None,
            }),
            None,
            Some(section::Deployment {
                model_name: "fast".to_owned(),
                config: None,
                preferences: Some(section::Preferences {
                    quality_tier: 3,
                    strengths: vec!["writing".to_owned()],
                }),
            }),
        ],
    };
    let resolved = resolve(&settings).unwrap();
    assert_eq!(resolved.len(), 3);
    let router = resolved[0].as_ref().unwrap();
    assert_eq!(router.model_name, "router-a");
    let weights = router.config.as_ref().unwrap().weights;
    assert_eq!(weights.quality.to_bits(), 0.7_f64.to_bits());
    assert_eq!(weights.economy.to_bits(), 0.3_f64.to_bits());
    assert_eq!(router.preferences, None);
    assert!(resolved[1].is_none());
    let fast = resolved[2].as_ref().unwrap();
    assert_eq!(fast.model_name, "fast");
    assert_eq!(fast.config, None);
    let preferences = fast.preferences.as_ref().unwrap();
    assert_eq!(preferences.quality_tier, 3);
    assert_eq!(preferences.strengths, vec![RequestType::Writing]);
}
