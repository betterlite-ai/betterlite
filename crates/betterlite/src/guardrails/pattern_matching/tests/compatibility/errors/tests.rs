#![cfg(test)]

use super::{ContentFilterAction, ContentFilterPattern, compile};
use rstest::rstest;

#[rstest]
#[case::unicode_name_missing_brace(r"\Ninvalid")]
#[case::unicode_name_unclosed(r"\N{LATIN SMALL LETTER I")]
#[case::unicode_name_unknown(r"\N{UNKNOWN NAME}")]
#[case::named_unicode_i(r"\N{LATIN SMALL LETTER I}")]
#[case::class_named_unicode_i(r"[\N{LATIN SMALL LETTER I}]")]
#[case::named_unicode_alias(r"\N{LF}")]
#[case::incompatible_flags("(?au:i)")]
#[case::disable_ascii("(?-a:i)")]
#[case::disable_unicode("(?-u:i)")]
#[case::hex_incomplete(r"\x6")]
#[case::hex_invalid(r"\xgg")]
#[case::unicode_invalid(r"\uD800")]
#[case::unicode_too_large(r"\U00110000")]
#[case::octal_too_large(r"\400")]
#[case::octal_class_too_large(r"[\400]")]
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
