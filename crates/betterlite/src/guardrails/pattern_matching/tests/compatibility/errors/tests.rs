#![cfg(test)]

use super::{ContentFilterAction, ContentFilterPattern, compile};
use rstest::rstest;

#[rstest]
#[case::start_anchor_in_class(r"[\A]")]
#[case::nonboundary_in_class(r"[\B]")]
#[case::unclosed_group("(i")]
#[case::unmatched_close("i)")]
#[case::unclosed_comment("(?# i$")]
#[case::unclosed_name("(?P<id")]
#[case::unclosed_named_reference("(?P=idi")]
#[case::unclosed_condition("(?P<id>a)(?(id")]
#[case::unclosed_numeric_condition("(a)(?(1")]
#[case::unclosed_flags("(?i")]
#[case::unclosed_marker("(?")]
#[case::class_trailing_escape("[a\\")]
#[case::unclosed_class("[a")]
#[case::empty_class("[]")]
#[case::unclosed_negated_class("[^a")]
#[case::trailing_escape("x\\")]
#[case::strict_end_inside_class(r"[\Z]")]
fn pattern_matching_behavior(#[case] source: &str) {
    compile(&ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source.to_owned()),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .unwrap_err();
}
