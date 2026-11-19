#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::custom_labels_from_tags;
use crate::prometheus_label_projection::test_support::texts;

#[rstest]
#[case::exact(&["prod"], &["prod"], &[("tag_prod", "true")])]
#[case::no_match_without_a_star(&["staging"], &["prod"], &[("tag_prod", "false")])]
#[case::a_prefix_is_not_an_exact_match(&["production"], &["prod"], &[("tag_prod", "false")])]
#[case::wildcard_match(
    &["User-Agent: curl/7.68.0"],
    &["User-Agent: curl/*"],
    &[("tag_User_Agent__curl__", "true")]
)]
#[case::wildcard_miss(
    &["User-Agent: wget/1.0"],
    &["User-Agent: curl/*"],
    &[("tag_User_Agent__curl__", "false")]
)]
#[case::every_configured_tag(&["prod"], &["prod", "dev"], &[("tag_dev", "false"), ("tag_prod", "true")])]
#[case::no_configured_tags(&["prod"], &[], &[])]
#[case::no_request_tags(&[], &["prod"], &[("tag_prod", "false")])]
fn tag_labels_say_whether_each_configured_tag_matched(
    #[case] tags: &[&str],
    #[case] configured: &[&str],
    #[case] expected: &[(&str, &str)],
) {
    let labels = custom_labels_from_tags(&texts(tags), &texts(configured)).expect("valid patterns");
    let expected: BTreeMap<String, String> = expected
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(labels, expected);
}

#[test]
fn a_configured_pattern_matches_itself_first() {
    let labels =
        custom_labels_from_tags(&texts(&["prod*"]), &texts(&["prod*"])).expect("a valid pattern");
    assert_eq!(labels.get("tag_prod_"), Some(&"true".to_owned()));
}
