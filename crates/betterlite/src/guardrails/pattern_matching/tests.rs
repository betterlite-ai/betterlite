#![cfg(test)]

use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterPattern,
};
use rstest::rstest;

use super::compile;

#[rstest]
#[case::custom_case_insensitive("regex", None, Some("secret"), "SECRET", true)]
#[case::prebuilt_email("prebuilt", Some("email"), None, "a@example.com", true)]
#[case::prebuilt_context_words(
    "prebuilt",
    Some("nl_bsn_contextual"),
    None,
    "BSN one two three four five six seven eight nine",
    true
)]
#[case::prebuilt_missing_context("prebuilt", Some("nl_bsn_contextual"), None, "123456789", false)]
#[case::custom_no_match("regex", None, Some("secret"), "public", false)]
fn pattern_matching_behavior(
    #[case] kind: &str,
    #[case] name: Option<&str>,
    #[case] regex: Option<&str>,
    #[case] text: &str,
    #[case] found: bool,
) {
    let pattern = compile(&ContentFilterPattern {
        pattern_type: kind.to_owned(),
        pattern_name: name.map(str::to_owned),
        pattern: regex.map(str::to_owned),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .unwrap();
    assert_eq!(!pattern.find_spans(text).unwrap().is_empty(), found);
}

#[path = "tests/errors/tests.rs"]
mod errors;

#[test]
fn pattern_order_wins_over_text_order() {
    let patterns: Vec<_> = ["last", "first"]
        .iter()
        .map(|source| {
            compile(&ContentFilterPattern {
                pattern_type: "regex".to_owned(),
                pattern_name: None,
                pattern: Some((*source).to_owned()),
                name: Some((*source).to_owned()),
                action: ContentFilterAction::Block,
            })
            .unwrap()
        })
        .collect();
    assert_eq!(
        super::check_patterns("first last", &patterns).unwrap(),
        Some(("last", "last", ContentFilterAction::Block))
    );
    assert_eq!(super::check_patterns("none", &patterns).unwrap(), None);
}

#[path = "tests/limits/tests.rs"]
mod limits;

#[path = "tests/compatibility/tests.rs"]
mod compatibility;

#[path = "tests/iteration/tests.rs"]
mod iteration;
