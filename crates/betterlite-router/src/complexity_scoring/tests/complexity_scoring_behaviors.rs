use std::collections::BTreeMap;

use betterlite_config::sections::complexity_scoring as section;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{
    CONFIG_INVALID, CONFIG_VALID, blank, classify, corpus_config, fixed, resolved, strings, tiers,
    weights,
};
use crate::complexity_scoring::classify::{ClassificationOutcome, PromptInput, Scorer};
use crate::complexity_scoring::config::{Config, TierBoundaries};
use crate::complexity_scoring::context_fit::{RequestContextFit, allowed};
use crate::complexity_scoring::decision::{
    DecisionInput, RoutingDecision, RoutingDecisionCause, SavingsBaseline, build_routing_decision,
};
use crate::complexity_scoring::dimensions::{CustomDimension, ScoringMode, estimated_tokens};
use crate::complexity_scoring::error::ScoringError;
use crate::complexity_scoring::tier::{ComplexityTier, Tier, tier_severity_order};

fn default_boundaries() -> TierBoundaries {
    TierBoundaries {
        simple_medium: 0.15_f64,
        medium_complex: 0.35_f64,
        complex_reasoning: 0.6_f64,
    }
}

// the decision every record test starts from, as a macro: a function would fix the borrow lifetime too early
macro_rules! base_record {
    () => {
        DecisionInput {
            router_model_name: "complexity-router",
            routed_model: "gpt-4o",
            cause: RoutingDecisionCause::HeuristicScorer,
            conversation_continuing: true,
            tier: None,
            tier_label: None,
            score: None,
            boundaries: default_boundaries(),
            reasoning_override_min_score: 0.15_f64,
            signals: &[],
            matched_keyword: None,
            escalation_keyword: None,
            escalated: false,
            classifier_model: None,
            classifier_cost: None,
            context_original_tier: None,
            tier_litellm_params: None,
            savings_baseline: None,
            previous_decision: None,
        }
    };
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite test number"))
}

fn object(value: &Json) -> &IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object, got {value:?}")
        }
    }
}

fn rendered(decision: &RoutingDecision) -> String {
    serde_json::to_string(decision).expect("the decision serializes")
}

fn scored_with(config: Config, prompt: &str) -> ClassificationOutcome {
    Scorer::new(config)
        .expect("the test keywords and patterns compile")
        .score_and_classify(&PromptInput {
            prompt,
            system_prompt: None,
        })
        .expect("the test prompt scores")
}

#[rstest]
#[case::empty("", "SIMPLE", "-0.100000", &["short (0 tokens)"], RoutingDecisionCause::HeuristicScorer)]
#[case::hello("hello", "SIMPLE", "-0.150000", &["short (1 tokens)", "simple (hello)"], RoutingDecisionCause::HeuristicScorer)]
#[case::code_one(
    "please review the following content and tell me what matters api",
    "MEDIUM",
    "0.150000",
    &["code (api)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::code_three(
    "function class def",
    "MEDIUM",
    "0.200000",
    &["short (4 tokens)", "code (function, class, def)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::technical_two(
    "architecture distributed",
    "SIMPLE",
    "0.025000",
    &["short (6 tokens)", "technical (architecture, distributed)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::technical_four(
    "architecture distributed encryption authentication",
    "MEDIUM",
    "0.150000",
    &["short (12 tokens)", "technical (architecture, distributed, encryption)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::technical_five(
    "architecture distributed encryption authentication concurrency",
    "MEDIUM",
    "0.250000",
    &["technical (architecture, distributed, encryption)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::reasoning_two(
    "step by step think through",
    "REASONING",
    "0.150000",
    &["short (6 tokens)", "reasoning (step by step, think through)"],
    RoutingDecisionCause::ReasoningOverride
)]
#[case::reasoning_one(
    "please think through this",
    "SIMPLE",
    "0.075000",
    &["short (6 tokens)", "reasoning (think through)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::multi_step(
    "first do this then that",
    "SIMPLE",
    "-0.085000",
    &["short (5 tokens)", "multi-step"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::numbered_step(
    "step 1 prepare",
    "SIMPLE",
    "-0.085000",
    &["short (3 tokens)", "multi-step"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::list_step("1. prepare", "SIMPLE", "-0.085000", &["short (2 tokens)", "multi-step"], RoutingDecisionCause::HeuristicScorer)]
#[case::lettered_step("a) prepare", "SIMPLE", "-0.085000", &["short (2 tokens)", "multi-step"], RoutingDecisionCause::HeuristicScorer)]
#[case::four_questions(
    "a b c d? e f? g h? i j? k",
    "SIMPLE",
    "-0.090000",
    &["short (6 tokens)", "4 questions"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::three_questions(
    "a b? c d? e f?",
    "SIMPLE",
    "-0.100000",
    &["short (3 tokens)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::word_boundary(
    "capital terrorism",
    "SIMPLE",
    "-0.100000",
    &["short (4 tokens)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::phrase(
    "open the pull request now",
    "MEDIUM",
    "0.200000",
    &["short (6 tokens)", "code (request, pull request)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::upper_case(
    "FUNCTION CLASS API",
    "MEDIUM",
    "0.200000",
    &["short (4 tokens)", "code (function, class, api)"],
    RoutingDecisionCause::HeuristicScorer
)]
#[case::negative_score(
    "howdy hello hi",
    "SIMPLE",
    "-0.150000",
    &["short (3 tokens)", "simple (hello, hi)"],
    RoutingDecisionCause::HeuristicScorer
)]
fn the_seven_dimensions_score_the_reference_table(
    #[case] prompt: &str,
    #[case] tier: &str,
    #[case] score: &str,
    #[case] signals: &[&str],
    #[case] cause: RoutingDecisionCause,
) {
    let outcome = classify(&blank(), prompt, None);
    assert_eq!(outcome.tier.name(), tier);
    assert_eq!(fixed(outcome.score), score);
    assert_eq!(outcome.signals, strings(signals));
    assert_eq!(outcome.cause, cause);
}

#[test]
fn the_system_prompt_never_scores() {
    let system = "function class def const architecture distributed step by step";
    let with_system = classify(&blank(), "please review this", Some(system));
    let without_system = classify(&blank(), "please review this", None);
    assert_eq!(with_system, without_system);
    assert_eq!(with_system.signals, strings(&["short (4 tokens)"]));
}

#[rstest]
#[case::empty("", 0)]
#[case::one_word("abcd", 1)]
#[case::partial_word("abcdefg", 1)]
#[case::three_words("abcdefghijkl", 3)]
#[case::two_cjk("\u{53d1}\u{7968}", 0)]
#[case::six_cjk("\u{53d1}\u{7968}\u{53d1}\u{7968}\u{53d1}\u{7968}", 1)]
fn tokens_come_from_characters_divided_by_four(#[case] input: &str, #[case] expected: i64) {
    assert_eq!(estimated_tokens(input), expected);
}

#[test]
fn a_long_prompt_scores_for_its_token_count() {
    let just_over = "b".repeat(1604);
    let over = classify(&blank(), &just_over, None);
    assert_eq!(over.signals, strings(&["long (401 tokens)"]));
    assert_eq!(fixed(over.score), "0.100000");
    let at_limit = "b".repeat(1600);
    assert!(classify(&blank(), &at_limit, None).signals.is_empty());
}

#[test]
fn a_configured_threshold_map_moves_the_short_and_long_signals() {
    let mut input = blank();
    input.complexity_thresholds = Some(BTreeMap::from([
        ("simple".to_owned(), 10_i64),
        ("complex".to_owned(), 20_i64),
    ]));
    let short = classify(&input, &"a".repeat(24), None);
    assert_eq!(short.signals, strings(&["short (6 tokens)"]));
    let middle = classify(&input, &"a".repeat(80), None);
    assert!(middle.signals.is_empty());
    let long = classify(&input, &"a".repeat(84), None);
    assert_eq!(long.signals, strings(&["long (21 tokens)"]));
    assert_eq!(fixed(long.score), "0.100000");
}

#[test]
fn custom_keywords_replace_and_extend_the_baseline_lists() {
    let mut input = blank();
    input.code_keywords = Some(strings(&["widget"]));
    input.technical_keywords = Some(strings(&["gizmo"]));
    input.custom_technical_keywords = Some(strings(&["Sprocket", "gizmo"]));
    input.simple_keywords = Some(strings(&["howdy"]));
    let prompt =
        "please review the following content and tell me what matters widget gizmo sprocket howdy";
    let outcome = classify(&input, prompt, None);
    assert_eq!(outcome.tier.name(), "MEDIUM");
    assert_eq!(fixed(outcome.score), "0.225000");
    assert_eq!(
        outcome.signals,
        strings(&[
            "code (widget)",
            "technical (gizmo, Sprocket)",
            "simple (howdy)"
        ])
    );
}

#[test]
fn a_repeated_custom_keyword_keeps_its_first_position_and_last_spelling() {
    let mut input = blank();
    input.technical_keywords = Some(strings(&["gizmo"]));
    input.custom_technical_keywords = Some(strings(&["Cog", "Sprocket", "sprocket"]));
    let outcome = classify(&input, "gizmo cog sprocket", None);
    assert_eq!(
        outcome.signals,
        strings(&["short (4 tokens)", "technical (gizmo, Cog, sprocket)"])
    );
    assert_eq!(fixed(outcome.score), "0.025000");
}

#[test]
fn an_empty_keyword_list_is_used_as_given() {
    let mut input = blank();
    input.code_keywords = Some(Vec::new());
    let outcome = classify(&input, "function class def", None);
    assert_eq!(outcome.signals, strings(&["short (4 tokens)"]));
    assert_eq!(fixed(outcome.score), "-0.100000");
}

#[test]
fn a_phrase_keyword_is_a_literal_substring() {
    let mut input = blank();
    input.simple_keywords = Some(strings(&["what is 2+2?"]));
    let outcome = classify(&input, "please tell me what is 2+2? now", None);
    assert_eq!(
        outcome.signals,
        strings(&["short (7 tokens)", "simple (what is 2+2?)"])
    );
    assert_eq!(fixed(outcome.score), "-0.150000");
}

#[test]
fn a_cjk_keyword_matches_inside_running_text() {
    let mut input = blank();
    input.custom_technical_keywords = Some(strings(&["\u{53d1}\u{7968}", "\u{62a5}\u{9500}"]));
    let outcome = classify(
        &input,
        "\u{6211}\u{9700}\u{8981}\u{5f00}\u{53d1}\u{7968}\u{548c}\u{62a5}\u{9500}",
        None,
    );
    assert_eq!(
        outcome.signals,
        strings(&[
            "short (2 tokens)",
            "technical (\u{53d1}\u{7968}, \u{62a5}\u{9500})"
        ])
    );
    assert_eq!(fixed(outcome.score), "0.025000");
}

#[test]
fn a_single_word_keyword_misses_a_longer_word() {
    let missed = classify(&blank(), "capital terrorism", None);
    assert_eq!(missed.signals, strings(&["short (4 tokens)"]));
    let hit = classify(&blank(), "api", None);
    assert_eq!(hit.signals, strings(&["short (0 tokens)", "code (api)"]));
}

#[test]
fn the_reasoning_override_needs_two_markers_and_the_floor() {
    let mut weighted = blank();
    weighted.dimension_weights = Some(weights(&[("reasoningMarkers", 0.05_f64)]));
    let below_floor = classify(&weighted, "step by step think through", None);
    assert_eq!(below_floor.tier.name(), "SIMPLE");
    assert_eq!(below_floor.cause, RoutingDecisionCause::HeuristicScorer);
    weighted.reasoning_override_min_score = Some(0.0_f64);
    let overridden = classify(&weighted, "step by step think through", None);
    assert_eq!(overridden.tier.name(), "REASONING");
    assert_eq!(overridden.cause, RoutingDecisionCause::ReasoningOverride);
    assert_eq!(fixed(overridden.score), "0.050000");
}

#[test]
fn one_reasoning_marker_never_overrides() {
    let mut input = blank();
    input.dimension_weights = Some(weights(&[("reasoningMarkers", 1.0_f64)]));
    let outcome = classify(&input, "please think through this", None);
    assert_eq!(outcome.tier.name(), "REASONING");
    assert_eq!(outcome.cause, RoutingDecisionCause::HeuristicScorer);
    assert_eq!(fixed(outcome.score), "0.700000");
}

#[test]
fn the_resolved_config_carries_the_pinned_defaults() {
    let config = resolved(&blank());
    assert_eq!(
        (
            config.token_thresholds.simple,
            config.token_thresholds.complex
        ),
        (15, 400)
    );
    assert_eq!(fixed(config.tier_boundaries.simple_medium), "0.150000");
    assert_eq!(fixed(config.tier_boundaries.medium_complex), "0.350000");
    assert_eq!(fixed(config.tier_boundaries.complex_reasoning), "0.600000");
    assert_eq!(fixed(config.reasoning_override_min_score), "0.150000");
    assert_eq!(
        config.dimension_weights,
        weights(&[
            ("tokenCount", 0.1_f64),
            ("codePresence", 0.3_f64),
            ("reasoningMarkers", 0.25_f64),
            ("technicalTerms", 0.25_f64),
            ("simpleIndicators", 0.05_f64),
            ("multiStepPatterns", 0.03_f64),
            ("questionComplexity", 0.02_f64),
        ])
    );
    assert_eq!(
        (
            config.code_keywords.len(),
            config.reasoning_keywords.len(),
            config.technical_keywords.len(),
            config.simple_keywords.len(),
            config.multi_step_patterns.len(),
        ),
        (45, 19, 28, 26, 4)
    );
    assert_eq!(config.custom_technical_keywords, Vec::<String>::new());
    assert_eq!(config.custom_dimensions.len(), 0);
    assert_eq!(config.default_model, None);
}

#[rstest]
#[case::non_reasoning("NON_REASONING", Some(ComplexityTier::NonReasoning))]
#[case::simple("SIMPLE", Some(ComplexityTier::Simple))]
#[case::medium("MEDIUM", Some(ComplexityTier::Medium))]
#[case::complex("COMPLEX", Some(ComplexityTier::Complex))]
#[case::reasoning("REASONING", Some(ComplexityTier::Reasoning))]
#[case::lower_case("simple", None)]
#[case::custom("PREMIUM", None)]
fn a_built_in_name_is_recognised_exactly(
    #[case] name: &str,
    #[case] expected: Option<ComplexityTier>,
) {
    assert_eq!(ComplexityTier::built_in(name), expected);
}

#[test]
fn a_pipeline_tier_name_is_the_plain_name() {
    assert_eq!(Tier::BuiltIn(ComplexityTier::Complex).name(), "COMPLEX");
    assert_eq!(Tier::Named("PREMIUM".to_owned()).name(), "PREMIUM");
}

#[test]
fn the_severity_order_lists_every_built_in_tier() {
    assert_eq!(
        tier_severity_order(false),
        [
            ComplexityTier::Simple,
            ComplexityTier::Medium,
            ComplexityTier::Complex,
            ComplexityTier::Reasoning
        ]
    );
    assert_eq!(
        tier_severity_order(true),
        [
            ComplexityTier::NonReasoning,
            ComplexityTier::Simple,
            ComplexityTier::Medium,
            ComplexityTier::Complex,
            ComplexityTier::Reasoning
        ]
    );
}

#[rstest]
#[case::simple(ComplexityTier::Simple, "gpt-4o-mini")]
#[case::medium(ComplexityTier::Medium, "gpt-4o")]
#[case::complex(ComplexityTier::Complex, "claude-sonnet-4-20250514")]
#[case::reasoning(ComplexityTier::Reasoning, "claude-sonnet-4-20250514")]
#[case::non_reasoning_falls_to_medium(ComplexityTier::NonReasoning, "gpt-4o")]
fn the_default_tier_models_are_the_pinned_ones(#[case] tier: ComplexityTier, #[case] model: &str) {
    let config = resolved(&blank());
    let selected = config
        .single_model_for_tier(&Tier::BuiltIn(tier))
        .expect("the default tier map answers every tier");
    assert_eq!(selected, model);
}

#[test]
fn a_configured_tier_map_replaces_the_defaults() {
    let mut input = blank();
    input.tier_models = Some(tiers(&[("SIMPLE", &["small-model"])]));
    let config = resolved(&input);
    assert_eq!(
        config
            .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Simple))
            .unwrap(),
        "small-model"
    );
    let missing = config.single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Complex));
    assert!(matches!(missing, Err(ScoringError::NoTierModel { .. })));
    assert!(missing.unwrap_err().to_string().contains("COMPLEX"));
}

#[test]
fn the_default_model_answers_a_tier_with_no_model() {
    let mut input = blank();
    input.tier_models = Some(tiers(&[
        ("SIMPLE", &["small-model"]),
        ("MEDIUM", &["medium-model"]),
    ]));
    input.default_model = Some("fallback-model".to_owned());
    let config = resolved(&input);
    assert_eq!(
        config
            .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Complex))
            .unwrap(),
        "fallback-model"
    );
    assert_eq!(
        config
            .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Medium))
            .unwrap(),
        "medium-model"
    );
}

#[test]
fn a_missing_tier_model_is_an_error() {
    let mut input = blank();
    input.tier_models = Some(tiers(&[]));
    let config = resolved(&input);
    let error = config
        .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Complex))
        .unwrap_err();
    assert!(error.to_string().contains("COMPLEX"), "got {error}");
}

#[test]
fn a_pool_of_one_answers_like_a_bare_name() {
    let mut input = blank();
    input.tier_models = Some(BTreeMap::from([(
        "SIMPLE".to_owned(),
        section::TierModelValue::Pool(strings(&["only-model"])),
    )]));
    let config = resolved(&input);
    assert_eq!(
        config
            .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Simple))
            .unwrap(),
        "only-model"
    );
}

#[test]
fn a_pool_of_several_has_no_single_answer() {
    let mut input = blank();
    input.tier_models = Some(tiers(&[("SIMPLE", &["first-model", "second-model"])]));
    let config = resolved(&input);
    let error = config
        .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Simple))
        .unwrap_err();
    assert!(
        matches!(error, ScoringError::TierPool { .. }),
        "got {error}"
    );
    assert!(error.to_string().contains("SIMPLE"), "got {error}");
}

#[test]
fn the_decision_record_names_the_router_and_the_route() {
    let decision = build_routing_decision(&base_record!()).unwrap();
    assert_eq!(
        rendered(&decision),
        r#"{"router_model_name":"complexity-router","router_type":"complexity","routed_model":"gpt-4o","cause":"heuristic_scorer","conversation_continuing":true}"#
    );
}

#[test]
fn optional_facts_are_omitted_rather_than_nulled() {
    let decision = build_routing_decision(&base_record!()).unwrap();
    let absent = [
        "tier",
        "tier_label",
        "score",
        "tier_boundaries",
        "reasoning_override_min_score",
        "signals",
        "matched_keyword",
        "escalation_keyword",
        "escalated",
        "classifier_model",
        "classifier_cost",
        "context_escalated",
        "context_escalation_original_tier",
        "tier_litellm_params",
        "savings_baseline_model",
        "savings_baseline_deployment_id",
    ];
    assert!(
        absent.iter().all(|key| !decision.contains_key(*key)),
        "an optional fact was recorded"
    );
}

#[test]
fn a_score_carries_its_boundaries_and_floor() {
    let tier = Tier::BuiltIn(ComplexityTier::Medium);
    let mut input = base_record!();
    input.tier = Some(&tier);
    input.score = Some(0.15_f64);
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(
        rendered(&decision),
        r#"{"router_model_name":"complexity-router","router_type":"complexity","routed_model":"gpt-4o","cause":"heuristic_scorer","conversation_continuing":true,"tier":"MEDIUM","score":0.15,"tier_boundaries":{"simple_medium":0.15,"medium_complex":0.35,"complex_reasoning":0.6},"reasoning_override_min_score":0.15}"#
    );
}

#[test]
fn a_tier_label_is_stored_only_when_it_differs() {
    let tier = Tier::BuiltIn(ComplexityTier::Medium);
    let mut same = base_record!();
    same.tier = Some(&tier);
    same.tier_label = Some("MEDIUM");
    assert!(
        !build_routing_decision(&same)
            .unwrap()
            .contains_key("tier_label")
    );
    let mut different = base_record!();
    different.tier = Some(&tier);
    different.tier_label = Some("Balanced");
    assert_eq!(
        build_routing_decision(&different)
            .unwrap()
            .get("tier_label"),
        Some(&text("Balanced"))
    );
}

#[test]
fn an_escalation_ask_and_its_outcome_travel_together() {
    let mut asked = base_record!();
    asked.escalation_keyword = Some("urgent");
    asked.escalated = false;
    let decision = build_routing_decision(&asked).unwrap();
    assert_eq!(decision.get("escalation_keyword"), Some(&text("urgent")));
    assert_eq!(decision.get("escalated"), Some(&Json::Bool(false)));
    assert!(
        !build_routing_decision(&base_record!())
            .unwrap()
            .contains_key("escalated")
    );
}

#[test]
fn a_context_escalation_names_the_original_tier() {
    let original = Tier::BuiltIn(ComplexityTier::Simple);
    let mut input = base_record!();
    input.context_original_tier = Some(&original);
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(decision.get("context_escalated"), Some(&Json::Bool(true)));
    assert_eq!(
        decision.get("context_escalation_original_tier"),
        Some(&text("SIMPLE"))
    );
}

#[test]
fn the_savings_baseline_carries_its_deployment() {
    let mut input = base_record!();
    input.savings_baseline = Some(SavingsBaseline {
        model: "gpt-4o",
        deployment_id: Some("deployment-1"),
    });
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(
        decision.get("savings_baseline_model"),
        Some(&text("gpt-4o"))
    );
    assert_eq!(
        decision.get("savings_baseline_deployment_id"),
        Some(&text("deployment-1"))
    );
    let mut without = base_record!();
    without.savings_baseline = Some(SavingsBaseline {
        model: "gpt-4o",
        deployment_id: None,
    });
    let bare_decision = build_routing_decision(&without).unwrap();
    assert_eq!(
        bare_decision.get("savings_baseline_model"),
        Some(&text("gpt-4o"))
    );
    assert!(!bare_decision.contains_key("savings_baseline_deployment_id"));
}

#[test]
fn classifier_facts_carry_over_from_the_previous_decision() {
    let mut previous = RoutingDecision::new();
    previous.insert("classifier_model".to_owned(), text("judge"));
    previous.insert("classifier_cost".to_owned(), number(0.25_f64));
    previous.insert("heuristic_v2_forecast".to_owned(), text("0.4"));
    previous.insert("unrelated".to_owned(), text("dropped"));
    let mut input = base_record!();
    input.previous_decision = Some(&previous);
    input.classifier_model = Some("new-judge");
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(
        rendered(&decision),
        r#"{"classifier_model":"new-judge","classifier_cost":0.25,"heuristic_v2_forecast":"0.4","router_model_name":"complexity-router","router_type":"complexity","routed_model":"gpt-4o","cause":"heuristic_scorer","conversation_continuing":true}"#
    );
}

#[test]
fn tier_litellm_params_are_masked_and_only_stored_when_present() {
    let mut params = IndexMap::new();
    params.insert("api_key".to_owned(), text("plain-value"));
    params.insert("model".to_owned(), text("gpt-4o"));
    let mut input = base_record!();
    input.tier_litellm_params = Some(&params);
    let decision = build_routing_decision(&input).unwrap();
    let stored = object(decision.get("tier_litellm_params").unwrap());
    assert_eq!(stored.get("model"), Some(&text("gpt-4o")));
    assert_ne!(stored.get("api_key"), Some(&text("plain-value")));
    let empty = IndexMap::new();
    let mut bare = base_record!();
    bare.tier_litellm_params = Some(&empty);
    assert!(
        !build_routing_decision(&bare)
            .unwrap()
            .contains_key("tier_litellm_params")
    );
}

#[test]
fn signals_are_stored_as_a_list() {
    let signals = strings(&["short (2 tokens)", "code (api)"]);
    let mut input = base_record!();
    input.signals = &signals;
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(
        decision.get("signals"),
        Some(&Json::List(vec![
            text("short (2 tokens)"),
            text("code (api)")
        ]))
    );
}

#[test]
fn a_matched_keyword_is_recorded_when_present() {
    let mut input = base_record!();
    input.matched_keyword = Some("urgent");
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(decision.get("matched_keyword"), Some(&text("urgent")));
}

// LiteLLM writes the classifier cost as a float, so a whole number keeps the decimal point its float carries
#[test]
fn a_whole_classifier_cost_keeps_the_decimal_point_a_float_carries() {
    let mut input = base_record!();
    input.classifier_cost = Some(Decimal::ONE);
    let decision = build_routing_decision(&input).unwrap();
    assert_eq!(decision.get("classifier_cost"), Some(&number(1.0_f64)));
}

#[test]
fn a_context_fit_accepts_unknown_windows_and_known_windows_that_hold() {
    let windows = BTreeMap::from([
        ("small-window".to_owned(), Some(101_u64)),
        ("unknown-window".to_owned(), None),
    ]);
    let fit = RequestContextFit {
        windows,
        needed: Some(50_u64),
        buffer: 0.5_f64,
    };
    assert!(fit.accepts("small-window"));
    assert!(fit.accepts("unknown-window"));
    assert!(fit.accepts("unlisted-model"));
    let too_big = RequestContextFit {
        windows: BTreeMap::from([("small-window".to_owned(), Some(101_u64))]),
        needed: Some(51_u64),
        buffer: 0.5_f64,
    };
    assert!(!too_big.accepts("small-window"));
    let unknown_need = RequestContextFit {
        windows: BTreeMap::from([("small-window".to_owned(), Some(1_u64))]),
        needed: None,
        buffer: 0.5_f64,
    };
    assert!(unknown_need.accepts("small-window"));
}

#[test]
fn a_fit_filter_keeps_the_pool_order() {
    let models = strings(&["first", "second", "third"]);
    assert_eq!(allowed(&models, None), models);
    let filter = ["third", "first"].map(str::to_owned).into_iter().collect();
    assert_eq!(
        allowed(&models, Some(&filter)),
        strings(&["first", "third"])
    );
}

#[test]
fn a_custom_dimension_scores_by_its_saturation() {
    let mut config = resolved(&blank());
    config.custom_dimensions = vec![CustomDimension {
        name: "domain".to_owned(),
        weight: 0.1_f64,
        keywords: strings(&["widget", "WIDGET"]),
        patterns: Vec::new(),
        scoring_mode: ScoringMode::MatchCount,
    }];
    let outcome = scored_with(config, "widget widget widget");
    assert_eq!(
        outcome.signals,
        strings(&["short (5 tokens)", "custom (domain)"])
    );
    assert_eq!(fixed(outcome.score), "-0.050000");
}

#[test]
fn a_binary_custom_dimension_saturates_at_one_hit() {
    let mut config = resolved(&blank());
    config.custom_dimensions = vec![CustomDimension {
        name: "domain".to_owned(),
        weight: 0.1_f64,
        keywords: strings(&["widget"]),
        patterns: strings(&["WIDGET AGAIN"]),
        scoring_mode: ScoringMode::Binary,
    }];
    let outcome = scored_with(config, "widget widget again");
    assert_eq!(
        outcome.signals,
        strings(&["short (4 tokens)", "custom (domain)"])
    );
    assert_eq!(fixed(outcome.score), "0.000000");
}

#[test]
fn a_custom_pattern_reads_only_the_first_2048_characters() {
    let dimension = CustomDimension {
        name: "domain".to_owned(),
        weight: 1.0_f64,
        keywords: strings(&["target"]),
        patterns: strings(&["target"]),
        scoring_mode: ScoringMode::MatchCount,
    };
    let mut config = resolved(&blank());
    config.custom_dimensions = vec![dimension];
    let both = scored_with(config.clone(), &format!("{} target", "x".repeat(2040)));
    assert_eq!(fixed(both.score), "1.100000");
    let keyword_only = scored_with(config, &format!("{} target", "x".repeat(2050)));
    assert_eq!(fixed(keyword_only.score), "0.600000");
}

#[test]
fn a_custom_dimension_with_no_hit_is_not_scored() {
    let mut config = resolved(&blank());
    config.custom_dimensions = vec![CustomDimension {
        name: "domain".to_owned(),
        weight: 1.0_f64,
        keywords: strings(&["nomatch"]),
        patterns: Vec::new(),
        scoring_mode: ScoringMode::MatchCount,
    }];
    let outcome = scored_with(config, "gizmo cog");
    assert_eq!(outcome.signals, strings(&["short (2 tokens)"]));
    assert_eq!(fixed(outcome.score), "-0.100000");
}

#[test]
fn a_non_finite_score_cannot_be_recorded() {
    let mut input = base_record!();
    input.score = Some(f64::INFINITY);
    let error = build_routing_decision(&input).unwrap_err();
    assert!(
        matches!(error, ScoringError::NonFinite { .. }),
        "got {error}"
    );
    assert!(error.to_string().contains("score"), "got {error}");
}

#[test]
fn the_valid_corpus_seed_loads_and_the_invalid_one_is_refused() {
    let config = corpus_config(CONFIG_VALID).expect("the valid seed loads");
    assert_eq!(
        config
            .single_model_for_tier(&Tier::BuiltIn(ComplexityTier::Simple))
            .unwrap(),
        "gpt-4o-mini"
    );
    assert_eq!(config.token_thresholds.simple, 15);
    let refused = corpus_config(CONFIG_INVALID).unwrap_err();
    assert!(refused.contains("complexity_thresholds"), "got {refused}");
}
