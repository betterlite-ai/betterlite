#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{CompiledPattern, CompiledRules, matches_regex, rule_matches_tool};
use crate::tool_permission::rule::{ToolDecision, ToolPermissionRule};

fn permission_rule(
    id: &str,
    tool_name: Option<&str>,
    tool_type: Option<&str>,
    allowed_param_patterns: Option<BTreeMap<String, String>>,
) -> ToolPermissionRule {
    ToolPermissionRule {
        id: id.to_owned(),
        tool_name: tool_name.map(str::to_owned),
        tool_type: tool_type.map(str::to_owned),
        decision: ToolDecision::Allow,
        allowed_param_patterns,
    }
}

fn compiled(rules: &[ToolPermissionRule]) -> CompiledRules {
    let mut compiled = CompiledRules::default();
    for rule in rules {
        compiled.compile_rule(rule).expect("a valid pattern");
    }
    compiled
}

#[test]
fn matches_regex_without_a_pattern_matches_anything() {
    assert!(matches_regex(None, Some("read_file")));
    assert!(matches_regex(None, None));
}

#[test]
fn matches_regex_without_a_value_never_matches() {
    let pattern = CompiledPattern::compile("^read$").unwrap();
    assert!(!matches_regex(Some(&pattern), None));
}

#[rstest]
#[case::whole_value("^read$", "read", true)]
#[case::prefix_only("^read$", "read_file", false)]
#[case::suffix_only("read$", "unread", false)]
#[case::pattern_not_covering_all("read", "read more", false)]
#[case::empty_pattern_matches_empty("", "", true)]
#[case::empty_pattern_rejects_text("", "read", false)]
#[case::everything(".*", "read_file", true)]
fn matches_regex_requires_the_whole_value(
    #[case] source: &str,
    #[case] value: &str,
    #[case] expected: bool,
) {
    let pattern = CompiledPattern::compile(source).unwrap();
    assert_eq!(matches_regex(Some(&pattern), Some(value)), expected);
}

#[rstest]
#[case::second_alternative("a|ab", "ab", true)]
#[case::first_alternative("a|ab", "a", true)]
#[case::no_alternative("a|ab", "b", false)]
fn matches_regex_fullmatches_each_alternative(
    #[case] source: &str,
    #[case] value: &str,
    #[case] expected: bool,
) {
    let pattern = CompiledPattern::compile(source).unwrap();
    assert_eq!(matches_regex(Some(&pattern), Some(value)), expected);
}

#[test]
fn rule_matches_tool_requires_the_name_pattern() {
    let rule = permission_rule("read", Some("^read$"), None, None);
    let compiled = compiled(std::slice::from_ref(&rule));
    assert_eq!(
        rule_matches_tool(&compiled, &rule, Some("read"), None),
        (true, true)
    );
    assert_eq!(
        rule_matches_tool(&compiled, &rule, Some("write"), None),
        (false, false)
    );
}

#[test]
fn rule_matches_tool_requires_the_type_pattern_too() {
    let rule = permission_rule("read", Some("^read$"), Some("function"), None);
    let compiled = compiled(std::slice::from_ref(&rule));
    assert_eq!(
        rule_matches_tool(&compiled, &rule, Some("read"), Some("function")),
        (true, true)
    );
    // the name still matched, so a later argument check would apply to this rule
    assert_eq!(
        rule_matches_tool(&compiled, &rule, Some("read"), Some("mcp")),
        (false, true)
    );
}

#[rstest]
#[case::type_matches("function", true)]
#[case::type_mismatch("mcp", false)]
fn rule_matches_tool_without_a_name_pattern_skips_parameter_checks(
    #[case] tool_type: &str,
    #[case] expected_match: bool,
) {
    let typed = permission_rule("typed", None, Some("^function$"), None);
    let compiled = compiled(std::slice::from_ref(&typed));
    assert_eq!(
        rule_matches_tool(&compiled, &typed, Some("anything"), Some(tool_type)),
        (expected_match, false)
    );
}

#[test]
fn rule_matches_tool_fails_when_the_name_is_absent() {
    let rule = permission_rule("read", Some("^read$"), None, None);
    let compiled = compiled(std::slice::from_ref(&rule));
    assert_eq!(
        rule_matches_tool(&compiled, &rule, None, None),
        (false, false)
    );
}

#[test]
fn duplicate_rule_ids_use_the_last_compiled_target() {
    let first = permission_rule("dup", Some("^first$"), None, None);
    let second = permission_rule("dup", Some("^second$"), None, None);
    let compiled = compiled(&[first.clone(), second]);
    let targets = compiled.targets.get("dup").unwrap();
    assert_eq!(targets.tool_name.as_ref().unwrap().source, "^second$");
    assert_eq!(
        rule_matches_tool(&compiled, &first, Some("first"), None),
        (false, false)
    );
}

#[test]
fn a_duplicate_rule_without_parameter_patterns_keeps_the_earlier_patterns() {
    let first = permission_rule(
        "dup",
        Some("^first$"),
        None,
        Some(BTreeMap::from([("path".to_owned(), "^/safe$".to_owned())])),
    );
    let second = permission_rule("dup", Some("^second$"), None, None);
    let compiled = compiled(&[first, second]);
    let patterns = compiled.patterns.get("dup").unwrap();
    assert_eq!(patterns.get("path").unwrap().source, "^/safe$");
    assert_eq!(
        compiled
            .targets
            .get("dup")
            .unwrap()
            .tool_name
            .as_ref()
            .unwrap()
            .source,
        "^second$"
    );
}

#[test]
fn compiled_pattern_keeps_the_rule_source() {
    let pattern = CompiledPattern::compile("^read_.*$").unwrap();
    assert_eq!(pattern.source, "^read_.*$");
}
