use std::collections::BTreeMap;

use rstest::rstest;

use super::{blank, classify, fixed, resolve, resolved, strings, weights};
use crate::complexity_math::{
    FULL, HALF, LOW, NEGATIVE, tier_for, weighted_score, window_holds, within_boundary_margin,
};
use crate::complexity_scoring::classify::{PromptInput, Scorer};
use crate::complexity_scoring::config::{TierBoundaries, TokenThresholds};
use crate::complexity_scoring::dimensions::{
    CODE_PRESENCE, CustomDimension, Dimension, DimensionScore, KeywordRule, REASONING_MARKERS,
    SIMPLE_INDICATORS, ScoringMode, TECHNICAL_TERMS, score_keyword_match, score_multi_step,
    score_question_complexity, score_token_count,
};
use crate::complexity_scoring::error::ScoringError;
use crate::complexity_scoring::keywords::{KeywordList, compile_patterns};
use crate::complexity_scoring::tier::ComplexityTier;

fn default_boundaries() -> TierBoundaries {
    TierBoundaries {
        simple_medium: 0.15,
        medium_complex: 0.35,
        complex_reasoning: 0.6,
    }
}

fn default_thresholds() -> TokenThresholds {
    TokenThresholds {
        simple: 15,
        complex: 400,
    }
}

fn equal_boundaries(
    simple_medium: f64,
    medium_complex: f64,
    complex_reasoning: f64,
) -> TierBoundaries {
    TierBoundaries {
        simple_medium,
        medium_complex,
        complex_reasoning,
    }
}

#[rstest]
#[case::negative(-1.0, ComplexityTier::Simple)]
#[case::just_below_simple_medium(0.149_999_999, ComplexityTier::Simple)]
#[case::at_simple_medium(0.15, ComplexityTier::Medium)]
#[case::just_below_medium_complex(0.349_999_999, ComplexityTier::Medium)]
#[case::at_medium_complex(0.35, ComplexityTier::Complex)]
#[case::just_below_complex_reasoning(0.599_999_999, ComplexityTier::Complex)]
#[case::at_complex_reasoning(0.6, ComplexityTier::Reasoning)]
#[case::far_above(10.0, ComplexityTier::Reasoning)]
fn a_score_on_a_boundary_takes_the_tier_above_it(
    #[case] score: f64,
    #[case] expected: ComplexityTier,
) {
    assert_eq!(tier_for(score, default_boundaries()), expected);
}

// the reference values: one code keyword scores 0.5, two score 1.0, and four technical terms add 0.25
#[rstest]
#[case::at_simple_medium(
    "please review the following content and tell me what matters api",
    "MEDIUM",
    &[("simple_medium", 0.5_f64), ("medium_complex", 1.0), ("complex_reasoning", 1.25)]
)]
#[case::at_medium_complex(
    "please review the following content and tell me what matters api sql",
    "COMPLEX",
    &[("simple_medium", 0.5_f64), ("medium_complex", 1.0), ("complex_reasoning", 1.25)]
)]
#[case::at_complex_reasoning(
    "please review the following content and tell me what matters api sql architecture distributed encryption authentication",
    "REASONING",
    &[("simple_medium", 0.5_f64), ("medium_complex", 1.0), ("complex_reasoning", 1.25)]
)]
#[case::below_simple_medium(
    "please review the following content and tell me what matters api",
    "SIMPLE",
    &[("simple_medium", 0.500_001_f64), ("medium_complex", 1.000_001), ("complex_reasoning", 1.250_001)]
)]
#[case::below_medium_complex(
    "please review the following content and tell me what matters api sql",
    "MEDIUM",
    &[("simple_medium", 0.500_001_f64), ("medium_complex", 1.000_001), ("complex_reasoning", 1.250_001)]
)]
#[case::below_complex_reasoning(
    "please review the following content and tell me what matters api sql architecture distributed encryption authentication",
    "COMPLEX",
    &[("simple_medium", 0.500_001_f64), ("medium_complex", 1.000_001), ("complex_reasoning", 1.250_001)]
)]
fn a_score_on_a_boundary_takes_the_tier_above_it_and_a_hair_below_keeps_the_lower(
    #[case] prompt: &str,
    #[case] tier: &str,
    #[case] boundaries: &[(&str, f64)],
) {
    let mut input = blank();
    input.dimension_weights = Some(weights(&[("codePresence", 1.0), ("technicalTerms", 0.25)]));
    input.tier_boundaries = Some(
        boundaries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect(),
    );
    let outcome = classify(&input, prompt, None);
    assert_eq!(outcome.tier.name(), tier);
}

#[rstest]
#[case::fourteen(14, "-1.000000", Some("short (14 tokens)"))]
#[case::fifteen(15, "0.000000", None)]
#[case::four_hundred(400, "0.000000", None)]
#[case::four_hundred_one(401, "1.000000", Some("long (401 tokens)"))]
fn a_token_count_at_a_threshold_is_neither_short_nor_long(
    #[case] tokens: i64,
    #[case] score: &str,
    #[case] signal: Option<&str>,
) {
    let scored = score_token_count(tokens, default_thresholds());
    assert_eq!(fixed(scored.score), score);
    assert_eq!(scored.signal.as_deref(), signal);
}

#[rstest]
#[case::code_none(&[] as &[&str], "nothing here", ("0.000000", None), 0, CODE_PRESENCE)]
#[case::code_one(&["api"], "call the api", ("0.500000", Some("code (api)")), 1, CODE_PRESENCE)]
#[case::code_two(&["api", "sql"], "api and sql", ("1.000000", Some("code (api, sql)")), 2, CODE_PRESENCE)]
#[case::code_three(
    &["api", "sql", "function"],
    "api sql function",
    ("1.000000", Some("code (api, sql, function)")),
    3,
    CODE_PRESENCE
)]
#[case::reasoning_none(&[] as &[&str], "nothing here", ("0.000000", None), 0, REASONING_MARKERS)]
#[case::reasoning_one(
    &["think through"],
    "think through this",
    ("0.700000", Some("reasoning (think through)")),
    1,
    REASONING_MARKERS
)]
#[case::reasoning_two(
    &["step by step", "think through"],
    "step by step think through",
    ("1.000000", Some("reasoning (step by step, think through)")),
    2,
    REASONING_MARKERS
)]
#[case::technical_below(
    &["architecture"],
    "architecture",
    ("0.000000", None),
    1,
    TECHNICAL_TERMS
)]
#[case::technical_at_low(
    &["architecture", "distributed"],
    "architecture distributed",
    ("0.500000", Some("technical (architecture, distributed)")),
    2,
    TECHNICAL_TERMS
)]
#[case::technical_between(
    &["architecture", "distributed", "encryption"],
    "architecture distributed encryption",
    ("0.500000", Some("technical (architecture, distributed, encryption)")),
    3,
    TECHNICAL_TERMS
)]
#[case::technical_high(
    &["architecture", "distributed", "encryption", "authentication"],
    "architecture distributed encryption authentication",
    ("1.000000", Some("technical (architecture, distributed, encryption)")),
    4,
    TECHNICAL_TERMS
)]
#[case::simple_none(&[] as &[&str], "nothing here", ("0.000000", None), 0, SIMPLE_INDICATORS)]
#[case::simple_one(&["hello"], "hello there", ("-1.000000", Some("simple (hello)")), 1, SIMPLE_INDICATORS)]
#[case::simple_two(
    &["hello", "thanks"],
    "hello and thanks",
    ("-1.000000", Some("simple (hello, thanks)")),
    2,
    SIMPLE_INDICATORS
)]
fn the_keyword_rules_score_at_their_thresholds(
    #[case] keywords: &[&str],
    #[case] text: &str,
    #[case] expected: (&str, Option<&str>),
    #[case] count: usize,
    #[case] rule: KeywordRule,
) {
    let compiled = KeywordList::compile(&strings(keywords)).unwrap();
    let (scored, matched) = score_keyword_match(text, &compiled, &rule);
    assert_eq!(fixed(scored.score), expected.0);
    assert_eq!(scored.signal.as_deref(), expected.1);
    assert_eq!(matched, count);
}

#[rstest]
#[case::three("a b? c d? e f?", "0.000000", None)]
#[case::four("a b? c d? e f? g h?", "0.500000", Some("4 questions"))]
fn a_question_count_past_three_counts_as_complex(
    #[case] text: &str,
    #[case] score: &str,
    #[case] signal: Option<&str>,
) {
    let scored = score_question_complexity(text);
    assert_eq!(fixed(scored.score), score);
    assert_eq!(scored.signal.as_deref(), signal);
}

#[test]
fn a_multi_step_pattern_scores_half() {
    let patterns = compile_patterns(&strings(&["(?i)first.*?then"])).unwrap();
    let hit = score_multi_step("first do this then that", &patterns);
    assert_eq!(fixed(hit.score), "0.500000");
    assert_eq!(hit.signal.as_deref(), Some("multi-step"));
    let missed = score_multi_step("do this before that", &patterns);
    assert_eq!(fixed(missed.score), "0.000000");
    assert_eq!(missed.signal, None);
}

#[test]
fn the_weighted_score_adds_the_custom_dimensions_last() {
    let built_in = [
        DimensionScore {
            dimension: Dimension::TokenCount,
            score: NEGATIVE,
            signal: None,
        },
        DimensionScore {
            dimension: Dimension::CodePresence,
            score: HALF,
            signal: None,
        },
        DimensionScore {
            dimension: Dimension::ReasoningMarkers,
            score: LOW,
            signal: None,
        },
    ];
    let custom = [(
        DimensionScore {
            dimension: Dimension::Custom("domain".to_owned()),
            score: FULL,
            signal: None,
        },
        0.25,
    )];
    let score = weighted_score(
        &built_in,
        &weights(&[("tokenCount", 0.1), ("codePresence", 0.3)]),
        &custom,
    );
    assert_eq!(fixed(score), "0.300000");
    let none_scored = weighted_score(&built_in, &weights(&[]), &[]);
    assert_eq!(fixed(none_scored), "0.000000");
}

#[rstest]
#[case::at_the_margin(0.5, true)]
#[case::beyond_margin(0.500_001, false)]
#[case::at_a_boundary(20.0, true)]
#[case::between_boundaries(5.0, false)]
fn a_margin_holds_a_score_near_any_boundary(#[case] score: f64, #[case] expected: bool) {
    let boundaries = equal_boundaries(0.0, 10.0, 20.0);
    assert_eq!(within_boundary_margin(score, boundaries, 0.5), expected);
}

#[rstest]
#[case::unknown_window(None, Some(100), 0.5, true)]
#[case::unknown_need(Some(10), None, 0.5, true)]
#[case::exact_budget(Some(100), Some(50), 0.5, true)]
#[case::truncated_budget(Some(101), Some(50), 0.5, true)]
#[case::one_over(Some(101), Some(51), 0.5, false)]
#[case::whole_window(Some(100), Some(100), 1.0, true)]
#[case::whole_window_over(Some(100), Some(101), 1.0, false)]
#[case::zero_need_on_zero_window(Some(0), Some(0), 0.5, true)]
#[case::negative_buffer(Some(100), Some(1), -0.5, false)]
#[case::window_past_u32(Some(u64::MAX), Some(u64::MAX), 1.0, false)]
fn a_window_holds_a_request_within_its_buffered_budget(
    #[case] window: Option<u64>,
    #[case] needed: Option<u64>,
    #[case] buffer: f64,
    #[case] expected: bool,
) {
    assert_eq!(window_holds(window, needed, buffer), expected);
}

#[test]
fn a_non_finite_weight_is_rejected_by_name() {
    let mut input = blank();
    input.dimension_weights = Some(weights(&[("codePresence", f64::NAN)]));
    let error = resolve(&input).unwrap_err();
    assert!(
        matches!(error, ScoringError::NonFinite { .. }),
        "got {error}"
    );
    assert!(
        error.to_string().contains("dimension_weights.codePresence"),
        "got {error}"
    );
}

#[test]
fn a_non_finite_boundary_is_rejected_by_name() {
    let mut input = blank();
    input.tier_boundaries = Some(BTreeMap::from([(
        "simple_medium".to_owned(),
        f64::NEG_INFINITY,
    )]));
    let error = resolve(&input).unwrap_err();
    assert!(
        matches!(error, ScoringError::NonFinite { .. }),
        "got {error}"
    );
    assert!(
        error.to_string().contains("tier_boundaries.simple_medium"),
        "got {error}"
    );
}

#[test]
fn a_non_finite_floor_is_rejected_by_name() {
    let mut input = blank();
    input.reasoning_override_min_score = Some(f64::INFINITY);
    let error = resolve(&input).unwrap_err();
    assert!(
        matches!(error, ScoringError::NonFinite { .. }),
        "got {error}"
    );
    assert!(
        error.to_string().contains("reasoning_override_min_score"),
        "got {error}"
    );
}

#[test]
fn a_weighted_score_that_overflows_is_rejected() {
    let mut input = blank();
    input.dimension_weights = Some(weights(&[
        ("codePresence", 1e308),
        ("technicalTerms", 1e308),
    ]));
    let outcome = Scorer::new(resolved(&input))
        .unwrap()
        .score_and_classify(&PromptInput {
            prompt: "function class architecture distributed encryption authentication",
            system_prompt: None,
        });
    assert!(
        matches!(outcome, Err(ScoringError::NonFiniteScore)),
        "got {outcome:?}"
    );
}

#[test]
fn an_invalid_pattern_is_rejected() {
    let mut config = resolved(&blank());
    config.multi_step_patterns = strings(&["("]);
    let Err(error) = Scorer::new(config) else {
        panic!("the pattern should be refused");
    };
    assert!(matches!(error, ScoringError::Pattern { .. }), "got {error}");
    let mut custom = resolved(&blank());
    custom.custom_dimensions = vec![CustomDimension {
        name: "domain".to_owned(),
        weight: 0.1,
        keywords: Vec::new(),
        patterns: strings(&["("]),
        scoring_mode: ScoringMode::Binary,
    }];
    let Err(error) = Scorer::new(custom) else {
        panic!("the custom pattern should be refused");
    };
    assert!(matches!(error, ScoringError::Pattern { .. }), "got {error}");
}

#[test]
fn the_floor_tracks_the_simple_medium_boundary_unless_configured() {
    assert_eq!(
        fixed(resolved(&blank()).reasoning_override_min_score),
        "0.150000"
    );
    let mut shifted = blank();
    shifted.tier_boundaries = Some(BTreeMap::from([("simple_medium".to_owned(), 0.4_f64)]));
    assert_eq!(
        fixed(resolved(&shifted).reasoning_override_min_score),
        "0.400000"
    );
    let mut explicit = shifted.clone();
    explicit.reasoning_override_min_score = Some(0.0);
    assert_eq!(
        fixed(resolved(&explicit).reasoning_override_min_score),
        "0.000000"
    );
}

#[test]
fn a_partial_threshold_map_keeps_the_other_default() {
    let mut input = blank();
    input.complexity_thresholds = Some(BTreeMap::from([("simple".to_owned(), 5_i64)]));
    let config = resolved(&input);
    assert_eq!(config.token_thresholds.simple, 5);
    assert_eq!(config.token_thresholds.complex, 400);
}

#[test]
fn a_partial_boundary_map_fills_each_unset_key() {
    let mut input = blank();
    input.tier_boundaries = Some(BTreeMap::from([("medium_complex".to_owned(), 0.5_f64)]));
    let boundaries = resolved(&input).tier_boundaries;
    assert_eq!(fixed(boundaries.simple_medium), "0.150000");
    assert_eq!(fixed(boundaries.medium_complex), "0.500000");
    assert_eq!(fixed(boundaries.complex_reasoning), "0.600000");
}

#[test]
fn an_empty_weight_map_scores_every_dimension_at_zero() {
    let mut input = blank();
    input.dimension_weights = Some(BTreeMap::new());
    let outcome = classify(&input, "function class architecture distributed", None);
    assert_eq!(fixed(outcome.score), "0.000000");
    assert_eq!(outcome.tier.name(), "SIMPLE");
}

#[test]
fn an_unknown_dimension_weight_never_scores() {
    let mut input = blank();
    input.dimension_weights = Some(weights(&[("unknownDimension", 1.0)]));
    let outcome = classify(&input, "function class def", None);
    assert_eq!(fixed(outcome.score), "0.000000");
    assert!(
        outcome
            .signals
            .iter()
            .any(|signal| signal.starts_with("code"))
    );
}
