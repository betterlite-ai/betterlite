#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::custom_dimensions::pattern::{PatternProblem, pattern_work};
use crate::sections::custom_dimensions::{
    Config, DimensionProblem, ScoringMode, Settings, validate_dimension,
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
#[case::shared_prefix_alternation("abc|abx|abe", 7)]
#[case::assertion_alternation("^|$", 7)]
#[case::verbose_space("(?x)a b", 3)]
#[case::verbose_class("(?x)[a b]", 5)]
#[case::verbose_hash_class("(?x)[a#b]", 5)]
#[case::verbose_brace_gap("(?x)a{2, 3}", 7)]
#[case::verbose_brace_padding("(?x)a{ 2 }", 5)]
#[case::local_flag_only("(?x:a)", 3)]
#[case::local_flag_removal("(?-x:a)", 3)]
#[case::local_unicode_flag("(?u:a)", 3)]
#[case::local_flag_pair("(?i-m:a)", 3)]
#[case::repeated_local_flag("(?ii:a)", 3)]
#[case::removed_local_flag("(?-i:a)", 3)]
#[case::paired_local_flags("(?x-i:a)", 3)]
#[case::stacked_global_flags("(?i)(?m)a", 2)]
#[case::flag_after_verbose_space("(?x) (?i)a", 2)]
#[case::comment_group("(?#c)a", 2)]
#[case::escaped_comment_group("(?#a\\)b)c", 2)]
#[case::named_group("(?P<name>a)", 3)]
#[case::class_leading_close("[]]", 2)]
#[case::class_negated_leading_close("[^]]", 2)]
#[case::class_leading_dash("[--a]", 3)]
#[case::class_only_dashes("[--]", 2)]
#[case::class_escape_backspace("[\\b]", 2)]
#[case::class_backspace_range("[\\b-\\f]", 3)]
#[case::end_text_assertion("\\Za", 3)]
#[case::hex_literal("\\x61", 2)]
#[case::padded_repeat("a{02}", 5)]
#[case::class_negated_trailing_dash("[]-a]", 3)]
#[case::class_leading_dash_literal("[-a]", 4)]
#[case::class_trailing_dash("[a-]", 4)]
#[case::class_category_dash("[\\d-]", 4)]
#[case::class_same_endpoints("[a-a]", 3)]
#[case::class_duplicate_literals("[aa]", 2)]
#[case::class_negated_duplicates("[^aa]", 2)]
#[case::class_after_range_dash("[a-a-z]", 5)]
#[case::class_three_dashes("[---a]", 4)]
#[case::class_five_dashes("[-----a]", 5)]
#[case::class_six_dashes("[------a]", 5)]
#[case::escaped_angle_open("\\<a", 3)]
#[case::escaped_angle_close("\\>a", 3)]
#[case::escaped_non_ascii("\\é", 2)]
#[case::class_escaped_non_ascii("[\\é]", 2)]
#[case::escaped_unicode("\\u00e9", 2)]
#[case::escaped_long_unicode("\\U000000e9", 2)]
#[case::named_group_non_ascii("(?P<é>a)", 3)]
#[case::named_group_underscore("(?P<_a>a)", 3)]
#[case::verbose_comment_to_newline("(?x)a#c\nb", 3)]
#[case::verbose_long_comment("(?x)a#cd\nb", 3)]
#[case::verbose_escaped_hash("(?x)a\\#b", 4)]
#[case::verbose_escaped_space("(?x)a\\ b", 4)]
#[case::verbose_only_comment("(?x)(?#c)", 1)]
#[case::verbose_compact_repeat("(?x)a{2,3}", 7)]
#[case::verbose_class_range("(?x)[a-b]", 3)]
#[case::class_escaped_dash("[a\\-b]", 5)]
#[case::escaped_brace("a\\{", 3)]
#[case::escaped_brace_repeat("\\{2}", 4)]
#[case::escaped_close_brace("\\}", 2)]
#[case::empty_braces("{}", 3)]
#[case::trailing_empty_braces("a{}", 4)]
#[case::lazy_exact_repeat("a{64}?", 67)]
#[case::escaped_underscore("\\_", 2)]
#[case::class_escaped_underscore("[\\_]", 2)]
#[case::class_escaped_hyphen("[\\-]", 2)]
#[case::spliced_leading_branch("(?:a)|b", 4)]
#[case::spliced_trailing_branch("a|(?:b)", 4)]
#[case::category_alternation("\\d|a", 4)]
#[case::class_alternation("[ab]|[cd]", 6)]
#[case::literal_class_alternation("a|[bc]", 5)]
#[case::global_unicode_flag("(?u)", 1)]
#[case::stacked_verbose_flag("(?i)(?x)a", 2)]
#[case::verbose_then_flag("(?x)(?i)a", 2)]
#[case::flagged_comment("(?#c)(?x)a", 2)]
#[case::ignore_case_and_unicode_flags("(?iu:a)", 3)]
#[case::dot(".", 2)]
#[case::literal_hash("a#b", 4)]
#[case::verbose_alternation("(?x)a|bc", 8)]
#[case::verbose_non_ascii_space("(?x)a\u{00A0}b", 4)]
#[case::scoped_verbose_group_then_hash("(?x:a)#b", 5)]
#[case::global_flag_then_space("(?i) a", 3)]
#[case::scoped_verbose_space("(?x:a b)", 4)]
#[case::caret_after_negation("[^^]", 2)]
#[case::negated_dash_run("[^--a]", 4)]
#[case::invalid_brace_content("a{2x}", 6)]
#[case::multiple_repeat_commas("a{2,,3}", 8)]
#[case::invalid_bounded_content("a{2,3,4}", 9)]
fn accepted_patterns_report_their_measured_work(#[case] pattern: &str, #[case] expected: u64) {
    assert_eq!(
        pattern_work(pattern).map(|(_matcher, steps)| steps),
        Ok(expected)
    );
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
#[case::largest_representable_repeat("a{4294967294}", PatternProblem::Repeat)]
#[case::reversed("a{2,1}", PatternProblem::Syntax)]
#[case::nested_repeat("a**", PatternProblem::Syntax)]
#[case::unclosed_group("(a", PatternProblem::Syntax)]
#[case::unclosed_class("[a", PatternProblem::Syntax)]
#[case::bad_range("[a-\\d]", PatternProblem::Syntax)]
#[case::negative_lookahead("(?!a)b", PatternProblem::Syntax)]
#[case::possessive("a{1}+", PatternProblem::Unsupported)]
#[case::lazy_possessive("a{1}+?", PatternProblem::Syntax)]
#[case::named_group_angle("(?<name>a)", PatternProblem::Syntax)]
#[case::greed_swap_flag("(?U:a)", PatternProblem::Syntax)]
#[case::crlf_flag("(?R:a)", PatternProblem::Syntax)]
#[case::locale_flag("(?L:a)", PatternProblem::Syntax)]
#[case::repeated_added_flag("(?i-i:a)", PatternProblem::Syntax)]
#[case::double_removal("(?i-m-x:a)", PatternProblem::Syntax)]
#[case::empty_flag_spec("(?-)", PatternProblem::Syntax)]
#[case::global_removal("(?-i)", PatternProblem::Syntax)]
#[case::ascii_unicode_flags("(?au:a)", PatternProblem::Syntax)]
#[case::local_ascii_flag("(?a:a)", PatternProblem::Unsupported)]
#[case::ascii_removal_pair("(?a-i:a)", PatternProblem::Unsupported)]
#[case::double_flag_separator("(?a-i-:a)", PatternProblem::Syntax)]
#[case::global_ascii_flag("(?a)", PatternProblem::Unsupported)]
#[case::global_then_local("(?a)(?a:a)", PatternProblem::Unsupported)]
#[case::ascii_and_multiline_flags("(?am:a)", PatternProblem::Unsupported)]
#[case::ascii_and_ignore_case_flags("(?ai:a)", PatternProblem::Unsupported)]
#[case::duplicate_ascii_flag("(?aa:a)", PatternProblem::Unsupported)]
#[case::two_local_flag_groups("(?a:a)(?u:b)", PatternProblem::Unsupported)]
#[case::lowercase_end_text("\\z", PatternProblem::Syntax)]
#[case::braced_hex("\\x{41}", PatternProblem::Syntax)]
#[case::braced_unicode("\\u{41}", PatternProblem::Syntax)]
#[case::named_class_escape("\\p{L}", PatternProblem::Syntax)]
#[case::negated_named_class("\\P{L}", PatternProblem::Syntax)]
#[case::class_named_class("[\\p{L}]", PatternProblem::Syntax)]
#[case::class_end_text("[\\Z]", PatternProblem::Syntax)]
#[case::class_start_text("[\\A]", PatternProblem::Syntax)]
#[case::class_not_boundary("[\\B]", PatternProblem::Syntax)]
#[case::class_bell_escape("[\\e]", PatternProblem::Syntax)]
#[case::named_escape_bare("\\N", PatternProblem::Syntax)]
#[case::class_named_escape("[\\N]", PatternProblem::Syntax)]
#[case::g_group_reference("\\g<1>", PatternProblem::Syntax)]
#[case::k_group_reference("\\k<n>", PatternProblem::Syntax)]
#[case::unknown_named_backref("(?P=name)", PatternProblem::Syntax)]
#[case::short_hex("\\x4", PatternProblem::Syntax)]
#[case::empty_hex("\\x", PatternProblem::Syntax)]
#[case::short_unicode("\\u041", PatternProblem::Syntax)]
#[case::digit_group_name("(?P<1a>a)", PatternProblem::Syntax)]
#[case::dotted_group_name("(?P<a.b>a)", PatternProblem::Syntax)]
#[case::repeat_before_brace("a{2}{3}", PatternProblem::Syntax)]
#[case::counted_over_u32("a{4294967296}", PatternProblem::Syntax)]
#[case::counted_max("a{4294967295}", PatternProblem::Syntax)]
#[case::bounded_max("a{2,4294967295}", PatternProblem::Syntax)]
#[case::zero_to_max("a{0,4294967295}", PatternProblem::Syntax)]
#[case::max_to_max("a{4294967294,4294967295}", PatternProblem::Syntax)]
#[case::possessive_optional("a?+", PatternProblem::Unsupported)]
#[case::possessive_star("a*+", PatternProblem::Unsupported)]
#[case::possessive_plus("a++", PatternProblem::Unsupported)]
#[case::possessive_class("[ab]++", PatternProblem::Unsupported)]
#[case::possessive_group("(?:a)++", PatternProblem::Unsupported)]
#[case::possessive_flag_group("(?i:a)++", PatternProblem::Unsupported)]
#[case::possessive_exact("a{2}+", PatternProblem::Unsupported)]
#[case::double_plus_on_counted("a{2}++", PatternProblem::Syntax)]
#[case::lazy_then_plus_on_counted("a{2}+?", PatternProblem::Syntax)]
#[case::lazy_then_plus("a+?+", PatternProblem::Syntax)]
#[case::lazy_star_then_plus("a*?+", PatternProblem::Syntax)]
#[case::triple_plus("a+++", PatternProblem::Syntax)]
#[case::global_removal_pair("(?i-m)a", PatternProblem::Syntax)]
#[case::global_removal_only("(?-i)a", PatternProblem::Syntax)]
#[case::ascii_unicode_global("(?au)a", PatternProblem::Syntax)]
#[case::flag_after_literal("a(?x)b", PatternProblem::Syntax)]
#[case::flag_after_literal_in_verbose("(?x)a(?i)b", PatternProblem::Syntax)]
#[case::flag_after_literal_in_ignore_case("(?i)a(?x)b", PatternProblem::Syntax)]
#[case::flag_in_group("(?:(?i)a)", PatternProblem::Syntax)]
#[case::flag_in_capture("((?i)a)", PatternProblem::Syntax)]
#[case::flag_after_local_group("(?i:a)(?m)", PatternProblem::Syntax)]
#[case::empty_local_group_then_flag("(?i:)(?m)", PatternProblem::Syntax)]
#[case::flag_after_empty_group("(?:)(?x)a", PatternProblem::Syntax)]
#[case::flag_after_empty_capture("()(?x)a", PatternProblem::Syntax)]
#[case::verbose_star_after_space("(?x)a *", PatternProblem::Repeat)]
#[case::verbose_lazy_star_after_space("(?x)a *?", PatternProblem::Repeat)]
#[case::verbose_lazy_plus("(?x)a+?", PatternProblem::Repeat)]
#[case::repeated_word_boundary("\\b{2}", PatternProblem::Syntax)]
#[case::repeated_assertion("^*", PatternProblem::Syntax)]
#[case::flag_after_escape("\\d(?i)a", PatternProblem::Syntax)]
#[case::trailing_flag_dash("(?i-:a)", PatternProblem::Syntax)]
#[case::assertion_alternation_over_budget("a{0,3}b{0,63}(?:^|$)", PatternProblem::Work)]
fn rejected_patterns_report_the_pinned_problem(
    #[case] pattern: &str,
    #[case] expected: PatternProblem,
) {
    assert_eq!(
        pattern_work(pattern).map(|(_matcher, steps)| steps),
        Err(expected)
    );
}

#[rstest]
#[case::named_escape("\\N{LATIN SMALL LETTER A}", Err(PatternProblem::Syntax))]
#[case::class_named_escape("[\\N{LATIN SMALL LETTER A}]", Err(PatternProblem::Syntax))]
#[case::surrogate_escape("\\uD800", Err(PatternProblem::Syntax))]
#[case::class_surrogate_escape("[\\uD800]", Err(PatternProblem::Syntax))]
#[case::large_octal("\\400", Ok(2))]
#[case::large_class_octal("[\\400]", Ok(2))]
#[case::large_octal_777("\\777", Ok(2))]
#[case::large_class_octal_777("[\\777]", Ok(2))]
#[case::octal_three_digits("\\111", Ok(2))]
#[case::class_octal_three_digits("[\\111]", Ok(2))]
#[case::octal_nul("\\0", Ok(2))]
#[case::group_reference_before_group("\\1(a)", Ok(4))]
#[case::group_reference_after_group("(a)\\1", Ok(4))]
#[case::intersection_class("[a&&b]", Ok(5))]
#[case::symmetric_class("[a~~b]", Ok(5))]
#[case::posix_class("[[:alpha:]]", Ok(9))]
#[case::nested_class("[[a]]", Ok(5))]
#[case::double_hyphen_range("[a-b--c]", Err(PatternProblem::Unsupported))]
#[case::double_hyphen_open("[!--a]", Err(PatternProblem::Unsupported))]
#[case::double_hyphen_closed("[a--b]", Err(PatternProblem::Unsupported))]
#[case::dash_run_only("[---]", Ok(2))]
#[case::dash_run_four("[----]", Ok(2))]
#[case::dash_run_five("[-----]", Ok(2))]
#[case::dash_run_five_then_literal("[-----a]", Ok(5))]
#[case::dash_run_six_then_literal("[------a]", Ok(5))]
#[case::dash_run_trailing("[---a-]", Ok(4))]
#[case::lookahead("(?=a)b", Err(PatternProblem::Syntax))]
#[case::lookbehind("(?<=a)b", Err(PatternProblem::Syntax))]
#[case::atomic_group("(?>a)", Err(PatternProblem::Syntax))]
#[case::named_backref("(?P<n>a)(?P=n)", Err(PatternProblem::Syntax))]
#[case::conditional("(a)(?(1)b|c)", Err(PatternProblem::Syntax))]
#[case::combining_group_name("(?P<a\u{0301}>a)", Err(PatternProblem::Syntax))]
#[case::verbose_lazy_after_space("(?x)a* ?", Err(PatternProblem::Repeat))]
fn declared_grammar_deviations_keep_their_measured_values(
    #[case] pattern: &str,
    #[case] expected: Result<u64, PatternProblem>,
) {
    assert_eq!(
        pattern_work(pattern).map(|(_matcher, steps)| steps),
        expected
    );
}

#[test]
fn pattern_problems_carry_the_pinned_messages() {
    assert_eq!(PatternProblem::Syntax.message(), "is not a valid regex");
    assert_eq!(
        PatternProblem::Unsupported.message(),
        "contains an unsupported regex construct"
    );
    assert_eq!(
        PatternProblem::Repeat.message(),
        "requires a single character or class repeated at most 64 times; use {n,m} instead of *, + or {n,}"
    );
    assert_eq!(
        PatternProblem::Depth.message(),
        "nests deeper than 16 levels"
    );
    assert_eq!(
        PatternProblem::Work.message(),
        "exceeds the per-pattern regex work budget"
    );
}

#[test]
fn the_repeat_limit_is_inclusive() {
    assert_eq!(pattern_work("a{64}").map(|(_matcher, steps)| steps), Ok(67));
    assert_eq!(
        pattern_work("a{3,64}").map(|(_matcher, steps)| steps),
        Ok(128)
    );
    assert_eq!(
        pattern_work("a{65}").map(|(_matcher, steps)| steps),
        Err(PatternProblem::Repeat)
    );
}

#[test]
fn the_work_limit_is_inclusive() {
    let allowed = "a".repeat(2047);
    let refused = "a".repeat(2048);
    assert_eq!(
        pattern_work(&allowed).map(|(_matcher, steps)| steps),
        Ok(2048)
    );
    assert_eq!(
        pattern_work(&refused).map(|(_matcher, steps)| steps),
        Err(PatternProblem::Work)
    );
}

#[test]
fn the_depth_limit_is_inclusive() {
    let allowed = format!("{}a{}", "(".repeat(16), ")".repeat(16));
    let refused = format!("{}a{}", "(".repeat(17), ")".repeat(17));
    assert_eq!(
        pattern_work(&allowed).map(|(_matcher, steps)| steps),
        Ok(18)
    );
    assert_eq!(
        pattern_work(&refused).map(|(_matcher, steps)| steps),
        Err(PatternProblem::Depth)
    );
}

#[test]
fn the_pattern_input_limit_is_inclusive() {
    let allowed = format!("(?x){}a", " ".repeat(4091));
    let refused = format!("(?x){}a", " ".repeat(4092));
    assert_eq!(pattern_work(&allowed).map(|(_matcher, steps)| steps), Ok(2));
    assert_eq!(
        pattern_work(&refused).map(|(_matcher, steps)| steps),
        Err(PatternProblem::Syntax)
    );
}

#[test]
fn repeated_opening_braces_are_bounded_before_normalization() {
    assert_eq!(pattern_work(&"{".repeat(4097)), Err(PatternProblem::Syntax));
    assert_eq!(
        pattern_work(&"{".repeat(256)).map(|(_, steps)| steps),
        Ok(257)
    );
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
fn a_source_written_twice_keeps_one_matcher() {
    let config = dimension("domain", &[], &["ab", "ab"], "match_count");
    let checked = validate_dimension(&config).unwrap();
    assert_eq!(checked.matcher_patterns, strings(&["ab"]));
}

#[test]
fn sources_that_normalize_alike_keep_a_matcher_each() {
    let config = dimension("domain", &[], &["(?x)a b", "(?x)ab"], "match_count");
    let checked = validate_dimension(&config).unwrap();
    assert_eq!(checked.matcher_patterns, strings(&["(?x)ab", "(?:(?x)ab)"]));
}

#[rstest]
#[case::repeated_added("(?ii:a)", "(?i:a)")]
#[case::repeated_removed("(?i-mm:a)", "(?i-m:a)")]
#[case::repeated_verbose("(?xx:a)", "(?x:a)")]
fn a_repeated_flag_is_carried_once(#[case] pattern: &str, #[case] expected: &str) {
    assert_eq!(
        pattern_work(pattern).map(|(matcher, _steps)| matcher),
        Ok(expected.to_owned())
    );
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
    allowed.patterns.push("k".to_owned());
    assert_eq!(
        validate_dimension(&allowed).map(|_checked| ()),
        Err(DimensionProblem::MatcherLimits)
    );
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
