#![cfg(test)]

use rstest::rstest;

use super::{matches_pattern, scope_matches};
use crate::context::PolicyMatchContext;
use crate::values::PolicyScope;

fn patterns(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| String::from(*value)).collect()
}

#[test]
fn a_missing_value_matches_only_star() {
    assert!(matches_pattern(None, &patterns(&["*"])));
    assert!(!matches_pattern(None, &patterns(&["team-*"])));
    assert!(!matches_pattern(None, &[]));
}

#[test]
fn a_trailing_star_is_a_raw_prefix_match() {
    assert!(matches_pattern(Some("gpt-4o"), &patterns(&["gpt-*"])));
    assert!(matches_pattern(Some("gpt-"), &patterns(&["gpt-*"])));
    assert!(!matches_pattern(Some("gpt"), &patterns(&["gpt-*"])));
    assert!(matches_pattern(Some("gpt-4o"), &patterns(&["gpt-4o"])));
    assert!(!matches_pattern(Some("gpt-4o"), &patterns(&["gpt-4"])));
}

#[test]
fn a_star_in_the_middle_is_literal() {
    assert!(!matches_pattern(Some("gpt-4o"), &patterns(&["gpt*4o"])));
    assert!(matches_pattern(Some("gpt*4o"), &patterns(&["gpt*4o"])));
}

#[rstest]
#[case::no_scope_tags(None, None, true)]
#[case::no_scope_tags_with_context_tags(None, Some(vec!["prod"]), true)]
#[case::scope_tags_without_context_tags(Some(vec!["health-*"]), None, false)]
#[case::scope_tags_with_empty_context_tags(Some(vec!["health-*"]), Some(vec![]), false)]
#[case::a_matching_context_tag(Some(vec!["health-*"]), Some(vec!["prod", "health-check"]), true)]
#[case::no_matching_context_tag(Some(vec!["health-*"]), Some(vec!["prod"]), false)]
fn tags_are_checked_only_when_the_scope_has_tags(
    #[case] scope_tags: Option<Vec<&str>>,
    #[case] context_tags: Option<Vec<&str>>,
    #[case] expected: bool,
) {
    let scope = PolicyScope {
        teams: None,
        keys: None,
        models: None,
        tags: scope_tags.map(|tags| patterns(&tags)),
    };
    let context = PolicyMatchContext {
        team_alias: Some("team".to_owned()),
        key_alias: Some("key".to_owned()),
        model: Some("model".to_owned()),
        tags: context_tags.map(|tags| patterns(&tags)),
    };
    assert_eq!(scope_matches(&scope, &context), expected);
}

#[rstest]
#[case::every_dimension_matches("healthcare", "dev-key", "gpt-4o", true)]
#[case::wrong_team("other", "dev-key", "gpt-4o", false)]
#[case::wrong_key("healthcare", "prod-key", "gpt-4o", false)]
#[case::wrong_model("healthcare", "dev-key", "claude-3", false)]
fn scope_matches_every_dimension(
    #[case] team_alias: &str,
    #[case] key_alias: &str,
    #[case] model: &str,
    #[case] expected: bool,
) {
    let scope = PolicyScope {
        teams: Some(patterns(&["healthcare"])),
        keys: Some(patterns(&["dev-*"])),
        models: Some(patterns(&["gpt-4*"])),
        tags: Some(patterns(&["prod"])),
    };
    let context = PolicyMatchContext {
        team_alias: Some(team_alias.to_owned()),
        key_alias: Some(key_alias.to_owned()),
        model: Some(model.to_owned()),
        tags: Some(patterns(&["prod"])),
    };
    assert_eq!(scope_matches(&scope, &context), expected);
}

#[test]
fn absent_context_values_match_the_star_defaults() {
    let scope = PolicyScope {
        teams: None,
        keys: None,
        models: None,
        tags: None,
    };
    let context = PolicyMatchContext {
        team_alias: None,
        key_alias: None,
        model: None,
        tags: None,
    };
    assert!(scope_matches(&scope, &context));
}
