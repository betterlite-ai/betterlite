use std::collections::BTreeMap;
use std::slice;

use betterlite_config::sections::complexity_scoring as scoring_section;
use betterlite_config::sections::custom_dimensions as section;
use betterlite_config::sections::custom_dimensions::pattern::PatternProblem;
use rstest::rstest;
use serde::Deserialize;
use serde::de::value::{Error, StrDeserializer};

use super::{config, dim, named, run, weights};
use crate::complexity_scoring::classify::{PromptInput, Scorer};
use crate::complexity_scoring::config::Config;
use crate::complexity_scoring::decision::RoutingDecisionCause;
use crate::complexity_scoring::dimensions::{
    CustomDimension, CustomDimensionMatcher, ScoringMode, score_custom_dimensions,
};
use crate::custom_dimensions::{ClassifierType, DimensionsError, validate};

#[rstest]
#[case::heuristic("heuristic", ClassifierType::Heuristic, true)]
#[case::heuristic_v2("heuristic_v2", ClassifierType::HeuristicV2, false)]
#[case::llm("llm", ClassifierType::Llm, false)]
#[case::capability("capability", ClassifierType::Capability, false)]
#[case::llm_v2("llm_v2", ClassifierType::LlmV2, false)]
#[case::custom("custom", ClassifierType::Custom, false)]
#[case::heuristic_first("heuristic_first", ClassifierType::HeuristicFirst, true)]
#[case::hybrid("hybrid", ClassifierType::Hybrid, true)]
#[case::jev("jev", ClassifierType::Jev, false)]
fn classifier_names_parse_and_gate_dimensions(
    #[case] name: &str,
    #[case] expected: ClassifierType,
    #[case] permitted: bool,
) {
    assert_eq!(
        ClassifierType::deserialize(StrDeserializer::<Error>::new(name)).unwrap(),
        expected
    );
    assert!(validate(&[], name, &BTreeMap::new()).is_ok());
    let result = validate(&[named("domain")], name, &BTreeMap::new());
    assert_eq!(result.is_ok(), permitted);
    if !permitted {
        assert_eq!(result, Err(DimensionsError::Classifier));
    }
}

#[test]
fn dollar_continuation_deviation_is_explicit() {
    let declared = [config("domain", 0.8_f64, &[], &["a$\\n"], "binary")];
    let mut settings = Config::resolve(&scoring_section::Config::default()).unwrap();
    settings.custom_dimensions = validate(&declared, "heuristic", &BTreeMap::new())
        .unwrap()
        .dimensions;
    let outcome = Scorer::new(settings)
        .unwrap()
        .score_and_classify(&PromptInput {
            prompt: "a\n",
            system_prompt: None,
        })
        .unwrap();
    assert_eq!(outcome.tier.name(), "SIMPLE");
    assert_eq!(format!("{:.6}", outcome.score), "-0.100000");
    assert_eq!(outcome.signals, ["short (0 tokens)"]);
    assert_eq!(outcome.cause, RoutingDecisionCause::HeuristicScorer);
}

#[rstest]
#[case::absent(vec![], "heuristic", weights(&[]), Ok(0))]
#[case::empty_skips_the_classifier_gate(vec![], "heuristic_v2", weights(&[]), Ok(0))]
#[case::empty_with_an_llm_classifier(vec![], "llm", weights(&[]), Ok(0))]
#[case::empty_with_an_unknown_classifier(
    vec![],
    "bogus",
    weights(&[]),
    Err(DimensionsError::ClassifierType { value: "bogus".to_owned() })
)]
#[case::one_keyword_dimension(
    vec![dim("domain", &["alpha"], &[])],
    "heuristic",
    weights(&[]),
    Ok(0)
)]
#[case::a_classifier_that_refuses_custom_dimensions(
    vec![dim("domain", &["alpha"], &[])],
    "heuristic_v2",
    weights(&[]),
    Err(DimensionsError::Classifier)
)]
#[case::match_count_with_hybrid(
    vec![config("domain", 0.5_f64, &[], &["gamma"], "match_count")],
    "hybrid",
    weights(&[]),
    Ok(6)
)]
#[case::duplicate_names_after_case_folding(
    vec![named("Domain"), named("domain")],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Names)
)]
#[case::a_built_in_dimension_name(
    vec![named("tokenCount")],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Names)
)]
#[case::a_built_in_dimension_name_in_another_case(
    vec![named("TOKENCOUNT")],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Names)
)]
#[case::a_weight_that_belongs_to_dimension_weights(
    vec![named("domain")],
    "heuristic",
    weights(&[("domain", 0.1_f64)]),
    Err(DimensionsError::Weights)
)]
#[case::a_weight_that_belongs_to_dimension_weights_in_another_case(
    vec![named("domain")],
    "heuristic",
    weights(&[("DOMAIN", 0.1_f64)]),
    Err(DimensionsError::Weights)
)]
#[case::the_work_of_every_pattern_is_summed(
    vec![dim("domain", &[], &["a{2}", "[a-z]{64}"])],
    "heuristic_first",
    weights(&[]),
    Ok(136)
)]
#[case::an_invalid_pattern_is_reported(
    vec![dim("domain", &[], &["a{65}"])],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Dimension(section::DimensionProblem::Pattern {
        pattern: "a{65}".to_owned(),
        problem: PatternProblem::Repeat,
    }))
)]
#[case::an_invalid_scoring_mode_is_reported(
    vec![config("domain", 0.5_f64, &["alpha"], &[], "count")],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Dimension(section::DimensionProblem::ScoringMode))
)]
#[case::a_dimension_without_matchers(
    vec![config("domain", 0.5_f64, &[], &[], "binary")],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Dimension(section::DimensionProblem::BlankMatchers))
)]
#[case::an_invalid_name_is_reported(
    vec![named("1domain")],
    "heuristic",
    weights(&[]),
    Err(DimensionsError::Dimension(section::DimensionProblem::Name))
)]
fn custom_dimensions_behaviors(
    #[case] declared: Vec<section::Config>,
    #[case] classifier: &str,
    #[case] dimension_weights: BTreeMap<String, f64>,
    #[case] expected: Result<u64, DimensionsError>,
) {
    assert_eq!(run(&declared, classifier, &dimension_weights), expected);
}

#[test]
fn a_validated_dimension_keeps_its_fields_and_mode() {
    let declared = [config(
        "domain",
        0.8_f64,
        &["alpha"],
        &["gamma"],
        "match_count",
    )];
    let validated = validate(&declared, "heuristic", &BTreeMap::new()).unwrap();
    assert_eq!(validated.work, 6);
    assert_eq!(
        validated.dimensions,
        [CustomDimension {
            name: "domain".to_owned(),
            weight: 0.8_f64,
            keywords: vec!["alpha".to_owned()],
            patterns: vec!["gamma".to_owned()],
            scoring_mode: ScoringMode::MatchCount,
        }]
    );
}

fn scored_with(declared: &[section::Config], prompt: &str) -> Vec<(String, f64)> {
    let validated = validate(declared, "heuristic", &BTreeMap::new()).unwrap();
    let matcher = CustomDimensionMatcher::compile(&validated.dimensions[0])
        .expect("the test matcher compiles");
    score_custom_dimensions(prompt, &prompt.to_lowercase(), slice::from_ref(&matcher))
        .into_iter()
        .map(|(score, _weight)| (score.dimension.name().to_owned(), score.score))
        .collect()
}

fn scored(mode: &str, prompt: &str) -> Vec<(String, f64)> {
    let declared = [config(
        "domain",
        0.8_f64,
        &["alpha", "beta"],
        &["gamma", "[0-9]{2}"],
        mode,
    )];
    scored_with(&declared, prompt)
}

#[rstest]
#[case::end_anchor("a\\Z", "a")]
#[case::dollar_matches_before_one_final_newline("a$", "a\n")]
#[case::multiline_dollar_matches_before_each_newline("(?m)a$", "a\nb")]
#[case::class_ampersand("[a&&b]", "&")]
#[case::octal_three_digits("\\111", "I")]
#[case::implicit_minimum("a{,3}", "aa")]
#[case::scoped_dot_all("(?s)a.b", "a\nb")]
#[case::scoped_multiline("(?m)^b", "a\nb")]
fn accepted_patterns_keep_their_python_meaning(#[case] pattern: &str, #[case] prompt: &str) {
    let declared = [config("domain", 0.8_f64, &[], &[pattern], "binary")];
    assert_eq!(
        scored_with(&declared, prompt),
        [("domain".to_owned(), 1.0_f64)]
    );
}

#[rstest]
#[case::spellings_that_normalize_alike(&["(?x)a b", "(?x)ab"], "xxxxab", 1.0_f64)]
#[case::one_source_written_twice(&["ab", "ab"], "xxxxab", 0.5_f64)]
#[case::spellings_the_matcher_reads_apart(&["\\111", "\\x49"], "xxxxI", 1.0_f64)]
fn pattern_hits_count_distinct_sources(
    #[case] patterns: &[&str],
    #[case] prompt: &str,
    #[case] expected: f64,
) {
    let declared = [config("domain", 0.8_f64, &[], patterns, "match_count")];
    assert_eq!(
        scored_with(&declared, prompt),
        [("domain".to_owned(), expected)]
    );
}

#[test]
fn binary_scores_one_when_any_matcher_hits() {
    assert_eq!(
        scored("binary", "alpha beta gamma 42"),
        [("domain".to_owned(), 1.0_f64)]
    );
    assert!(scored("binary", "zeta").is_empty());
}

#[test]
fn match_count_scores_half_for_one_hit_and_one_for_two() {
    assert_eq!(
        scored("match_count", "alpha"),
        [("domain".to_owned(), 0.5_f64)]
    );
    assert_eq!(
        scored("match_count", "alpha beta"),
        [("domain".to_owned(), 1.0_f64)]
    );
}

#[test]
fn a_repeated_match_counts_once() {
    assert_eq!(
        scored("match_count", "alpha alpha"),
        [("domain".to_owned(), 0.5_f64)]
    );
}
