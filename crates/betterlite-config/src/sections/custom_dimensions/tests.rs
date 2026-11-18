#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::{files, rstest};

use crate::loader::ConfigLoader;
use crate::sections::custom_dimensions::{
    Config, PatternProblem, ScoringMode, Settings, pattern_work, validate_dimension,
};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.custom_dimensions)
        .map_err(|error| error.to_string())
}

fn deployment(keys: &str) -> String {
    format!(
        "model_list:\n  - model_name: primary\n    litellm_params:\n      complexity_router_config:\n        custom_dimensions:\n{keys}"
    )
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn dimension(name: &str, keywords: &[&str], patterns: &[&str], scoring_mode: &str) -> Config {
    Config {
        name: name.to_owned(),
        weight: 0.5,
        keywords: strings(keywords),
        patterns: strings(patterns),
        scoring_mode: scoring_mode.to_owned(),
    }
}

fn valid(config: &Config) -> Result<(), ()> {
    validate_dimension(config)
        .map(|_checked| ())
        .map_err(|_problem| ())
}

#[test]
fn a_config_without_custom_dimensions_has_no_deployments() {
    assert_eq!(settings("{}", &[]).unwrap(), Settings::default());
    assert_eq!(
        settings("model_list: []", &[]).unwrap(),
        Settings::default()
    );
}

#[test]
fn a_deployment_without_the_router_config_has_no_dimensions() {
    let source = "model_list:\n  - model_name: primary\n    litellm_params:\n      model: gpt-4o\n";
    assert_eq!(settings(source, &[]).unwrap().deployments, [Vec::new()]);
}

#[test]
fn a_deployment_reads_each_dimension_field() {
    let keys = "          - name: domain
            weight: 0.8
            keywords: [alpha, beta]
            patterns: ['gamma', '[0-9]{2}']
            scoring_mode: match_count
";
    let settings = settings(&deployment(keys), &[]).unwrap();
    assert_eq!(
        settings.deployments,
        [vec![Config {
            name: "domain".to_owned(),
            weight: 0.8,
            keywords: strings(&["alpha", "beta"]),
            patterns: strings(&["gamma", "[0-9]{2}"]),
            scoring_mode: "match_count".to_owned(),
        }]]
    );
}

#[test]
fn an_omitted_scoring_mode_defaults_to_binary() {
    let keys = "          - name: domain\n            weight: 0.5\n            keywords: [alpha]\n";
    let settings = settings(&deployment(keys), &[]).unwrap();
    assert_eq!(settings.deployments[0][0].scoring_mode, "binary");
}

#[test]
fn every_dimension_text_field_resolves_the_environment_once() {
    let keys = "          - name: os.environ/NAME
            weight: os.environ/WEIGHT
            keywords: [os.environ/KEYWORD]
            patterns: [os.environ/PATTERN]
            scoring_mode: os.environ/MODE
";
    let variables = [
        ("NAME", "domain"),
        ("WEIGHT", "0.8"),
        ("KEYWORD", "alpha"),
        ("PATTERN", "gamma"),
        ("MODE", "match_count"),
    ];
    let settings = settings(&deployment(keys), &variables).unwrap();
    assert_eq!(
        settings.deployments,
        [vec![Config {
            name: "domain".to_owned(),
            weight: 0.8,
            keywords: strings(&["alpha"]),
            patterns: strings(&["gamma"]),
            scoring_mode: "match_count".to_owned(),
        }]]
    );
}

#[test]
fn an_empty_environment_weight_reads_as_zero() {
    let keys = "          - name: domain\n            weight: os.environ/WEIGHT\n            keywords: [alpha]\n";
    let settings = settings(&deployment(keys), &[("WEIGHT", "")]).unwrap();
    assert!(settings.deployments[0][0].weight.abs() < f64::EPSILON);
}

#[test]
fn a_missing_environment_variable_names_the_field() {
    let keys = "          - name: domain\n            weight: os.environ/WEIGHT\n            keywords: [alpha]\n";
    let error = settings(&deployment(keys), &[]).unwrap_err();
    assert!(error.contains("WEIGHT"), "{error}");
    assert!(error.contains("weight"), "{error}");
}

#[rstest]
#[case::quoted_number(
    "          - name: domain\n            weight: '0.5'\n            keywords: [alpha]\n"
)]
#[case::unknown_field(
    "          - name: domain\n            weight: 0.5\n            keywords: [alpha]\n            colour: red\n"
)]
#[case::unknown_shape(
    "          - name: domain\n            weight: [0.5]\n            keywords: [alpha]\n"
)]
fn a_malformed_dimension_is_an_error(#[case] keys: &str) {
    assert!(settings(&deployment(keys), &[]).is_err());
}

#[rstest]
#[case::empty("", 1)]
#[case::literal("a", 2)]
#[case::exact_two("a{2}", 5)]
#[case::exact_ten("a{10}", 13)]
#[case::bounded("a{0,64}", 131)]
#[case::empty_range("a{0,0}", 3)]
#[case::implicit_min("a{,3}", 9)]
#[case::optional("a?", 5)]
#[case::lazy("a{1}?", 4)]
#[case::open_brace("{", 2)]
#[case::trailing_brace("a{", 3)]
#[case::class("[a-z]", 3)]
#[case::class_repeat("[a-z]{64}", 131)]
#[case::character_class("[abc]", 5)]
#[case::negated_class("[^abc]", 6)]
#[case::category("\\d", 3)]
#[case::category_repeat("\\d{2}", 7)]
#[case::bracketed_category("[\\d]{2}", 7)]
#[case::group("(ab)", 4)]
#[case::alternation("a|b", 4)]
#[case::hoisted_literal("a|a", 6)]
#[case::hoisted_prefix("a|ab", 7)]
#[case::merged_alternation("(?:a|b){2}", 9)]
#[case::spliced_group("(?:a){2}", 5)]
#[case::empty_capture("()", 2)]
#[case::assertion("^a", 3)]
#[case::word_boundary("\\ba", 3)]
#[case::global_flag_alternation("(?i)a|b", 4)]
#[case::escaped_literal("\\x61|a", 6)]
#[case::empty_branch("a|", 6)]
#[case::empty_capture_branches("(|)", 6)]
#[case::empty_group_branches("(?:|)", 5)]
#[case::optional_alternation("a?|a?", 13)]
#[case::shared_prefix_alternation("abc|abd|abe", 7)]
#[case::verbose_space("(?x)a b", 3)]
#[case::verbose_class("(?x)[a b]", 5)]
#[case::local_flag_only("(?x:a)", 3)]
#[case::local_flag_removal("(?-x:a)", 3)]
#[case::local_ascii_flag("(?a:a)", 3)]
#[case::local_unicode_flag("(?u:a)", 3)]
#[case::local_flag_pair("(?i-m:a)", 3)]
#[case::repeated_local_flag("(?ii:a)", 3)]
fn accepted_patterns_report_their_measured_work(#[case] pattern: &str, #[case] expected: u64) {
    assert_eq!(pattern_work(pattern), Ok(expected));
}

#[rstest]
#[case::never_closed("a{65}", PatternProblem::Repeat)]
#[case::star("a*", PatternProblem::Repeat)]
#[case::plus("a+", PatternProblem::Repeat)]
#[case::open_ended("a{2,}", PatternProblem::Repeat)]
#[case::implicit_open("a{,}", PatternProblem::Repeat)]
#[case::group_repeat("(?:ab){2}", PatternProblem::Repeat)]
#[case::branch_repeat("(?:a|bc){2}", PatternProblem::Repeat)]
#[case::capture_repeat("(a){2}", PatternProblem::Repeat)]
#[case::flag_group_repeat("(?i:a){2}", PatternProblem::Repeat)]
#[case::just_over_repeat("a{3,65}", PatternProblem::Repeat)]
#[case::too_large("a{4294967295}", PatternProblem::Syntax)]
#[case::reversed("a{2,1}", PatternProblem::Syntax)]
#[case::nested_repeat("a**", PatternProblem::Syntax)]
#[case::unclosed_group("(a", PatternProblem::Syntax)]
#[case::unclosed_class("[a", PatternProblem::Syntax)]
#[case::bad_range("[a-\\d]", PatternProblem::Syntax)]
#[case::invalid_reference("\\1(a)", PatternProblem::Syntax)]
#[case::lookahead("(?=a)b", PatternProblem::Unsupported)]
#[case::backreference("(a)\\1", PatternProblem::Unsupported)]
#[case::possessive("a{1}+", PatternProblem::Unsupported)]
fn rejected_patterns_report_the_pinned_problem(
    #[case] pattern: &str,
    #[case] expected: PatternProblem,
) {
    assert_eq!(pattern_work(pattern), Err(expected));
}

#[test]
fn the_repeat_limit_is_inclusive() {
    assert_eq!(pattern_work("a{64}"), Ok(67));
    assert_eq!(pattern_work("a{3,64}"), Ok(128));
    assert_eq!(pattern_work("a{65}"), Err(PatternProblem::Repeat));
}

#[test]
fn the_work_limit_is_inclusive() {
    let allowed = "a".repeat(2047);
    let refused = "a".repeat(2048);
    assert_eq!(pattern_work(&allowed), Ok(2048));
    assert_eq!(pattern_work(&refused), Err(PatternProblem::Work));
}

#[test]
fn the_depth_limit_is_inclusive() {
    let allowed = format!("{}a{}", "(".repeat(16), ")".repeat(16));
    let refused = format!("{}a{}", "(".repeat(17), ")".repeat(17));
    assert_eq!(pattern_work(&allowed), Ok(18));
    assert_eq!(pattern_work(&refused), Err(PatternProblem::Depth));
}

#[rstest]
#[case::keyword_only(vec!["alpha"], vec![], true)]
#[case::pattern_only(vec![], vec!["[0-9]{2}"], true)]
#[case::both_empty(vec![], vec![], false)]
#[case::blank_keyword(vec![" "], vec![], false)]
#[case::blank_pattern(vec![], vec!["\u{00A0}"], false)]
#[case::bad_pattern(vec![], vec!["a{65}"], false)]
fn dimension_validation_checks_the_matchers(
    #[case] keywords: Vec<&str>,
    #[case] patterns: Vec<&str>,
    #[case] expected: bool,
) {
    let config = dimension("domain", &keywords, &patterns, "binary");
    assert_eq!(valid(&config).is_ok(), expected);
}

#[test]
fn thirty_two_matchers_are_allowed_and_thirty_three_are_not() {
    let mut allowed = dimension("domain", &[], &[], "binary");
    allowed.keywords = (0..32).map(|index| format!("k{index}")).collect();
    assert_eq!(valid(&allowed), Ok(()));
    allowed.keywords.push("k32".to_owned());
    assert_eq!(valid(&allowed), Err(()));
}

#[test]
fn four_thousand_ninety_six_matcher_characters_are_allowed_and_one_more_is_not() {
    let mut allowed = dimension("domain", &[], &[], "binary");
    allowed.patterns = (0..16).map(|index| format!("{index:0<256}")).collect();
    assert_eq!(valid(&allowed), Ok(()));
    allowed.patterns[0].push('a');
    assert_eq!(valid(&allowed), Err(()));
}

#[rstest]
#[case::shortest(1, true)]
#[case::longest(256, true)]
#[case::too_long(257, false)]
#[case::empty(0, false)]
fn matcher_lengths_are_bounded(#[case] length: usize, #[case] expected: bool) {
    let config = Config {
        keywords: vec!["a".repeat(length)],
        patterns: Vec::new(),
        ..dimension("domain", &[], &[], "binary")
    };
    assert_eq!(valid(&config).is_ok(), expected);
}

#[test]
fn a_blank_keyword_is_blank_even_when_it_is_not_ascii_space() {
    let config = Config {
        keywords: vec!["\u{001C}".to_owned()],
        patterns: Vec::new(),
        ..dimension("domain", &[], &[], "binary")
    };
    assert_eq!(valid(&config), Err(()));
}

#[rstest]
#[case::binary("binary", Ok(ScoringMode::Binary))]
#[case::match_count("match_count", Ok(ScoringMode::MatchCount))]
#[case::count("count", Err(()))]
#[case::empty("", Err(()))]
fn scoring_mode_must_be_binary_or_match_count(
    #[case] mode: &str,
    #[case] expected: Result<ScoringMode, ()>,
) {
    let config = dimension("domain", &["alpha"], &[], mode);
    assert_eq!(
        validate_dimension(&config)
            .map(|checked| checked.scoring_mode)
            .map_err(|_problem| ()),
        expected
    );
}

#[test]
fn validate_dimension_carries_the_pattern_work_total() {
    let config = dimension("domain", &[], &["a{2}", "[a-z]{64}"], "binary");
    assert_eq!(validate_dimension(&config).unwrap().work, 136);
}

#[test]
fn the_config_corpus_seeds_load_or_are_refused() {
    let valid = include_str!("../../../fuzz/corpus/config_yaml/custom-dimensions-valid.yaml");
    assert!(settings(valid, &[]).is_ok());
    let invalid = include_str!("../../../fuzz/corpus/config_yaml/custom-dimensions-invalid.yaml");
    assert!(settings(invalid, &[]).is_err());
}

#[rstest]
fn replays_the_dimension_pattern_corpus(#[files("fuzz/corpus/dimension_pattern/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(pattern) = std::str::from_utf8(&bytes) else {
        return;
    };
    drop(pattern_work(pattern));
}
