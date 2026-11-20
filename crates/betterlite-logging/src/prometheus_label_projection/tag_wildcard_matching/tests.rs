#![cfg(test)]

use rstest::rstest;

use super::tag_matches_wildcard_pattern;
use crate::prometheus_label_projection::test_support::texts;

#[rstest]
#[case::exact_tag(&["curl"], "curl", true)]
#[case::prefix_matches_without_a_star(&["prefixed"], "pre", true)]
#[case::a_prefix_match_is_unrelated_to_its_position(&["xp"], "pre", false)]
fn the_pattern_is_anchored_at_the_start_only(
    #[case] tags: &[&str],
    #[case] configured: &str,
    #[case] expected: bool,
) {
    let matched = tag_matches_wildcard_pattern(&texts(tags), configured).expect("a valid pattern");
    assert_eq!(matched, expected);
}

#[rstest]
#[case::curl_agent("User-Agent: curl/*", &["User-Agent: curl/7.68.0"], true)]
#[case::empty_tail("curl/*", &["curl/"], true)]
#[case::other_agent("User-Agent: curl/*", &["User-Agent: wget/1.0"], false)]
#[case::one_matching_tag("prod", &["dev", "prod", "staging"], true)]
#[case::no_tags("prod", &[], false)]
#[case::several_stars("a*b*c", &["aXbYc"], true)]
#[case::stars_need_their_segments("a*b*c", &["aXc"], false)]
fn a_star_matches_any_text(
    #[case] configured: &str,
    #[case] tags: &[&str],
    #[case] expected: bool,
) {
    let matched = tag_matches_wildcard_pattern(&texts(tags), configured).expect("a valid pattern");
    assert_eq!(matched, expected);
}

#[rstest]
#[case::dot_does_not_match_another_character("a.b", &["axb"], false)]
#[case::dot_matches_itself("a.b", &["a.b"], true)]
#[case::plus_does_not_repeat("a+b", &["aab"], false)]
#[case::plus_matches_itself("a+b", &["a+b"], true)]
#[case::alternation_does_not_branch("(x)|y", &["x"], false)]
#[case::alternation_matches_itself("(x)|y", &["(x)|y"], true)]
fn regex_metacharacters_in_a_configured_tag_are_literal(
    #[case] configured: &str,
    #[case] tags: &[&str],
    #[case] expected: bool,
) {
    let matched = tag_matches_wildcard_pattern(&texts(tags), configured).expect("a valid pattern");
    assert_eq!(matched, expected);
}
