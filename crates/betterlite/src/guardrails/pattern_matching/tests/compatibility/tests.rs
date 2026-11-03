#![cfg(test)]

use std::fs;
use std::path::Path;

use super::{ContentFilterAction, ContentFilterPattern, compile};
use rstest::rstest;

#[rstest]
#[case::trailing_hyphen_lower("regex", "[i-]", "-", true)]
#[case::trailing_hyphen_upper("regex", "[I-]", "-", true)]
#[case::trailing_hyphen_no_range("regex", "[I-]", "[", false)]
#[case::negated_trailing_hyphen("regex", "[^i-]", "-", false)]
#[case::negated_upper_hyphen("regex", "[^I-]", "[", true)]
#[case::escaped_trailing_hyphen("regex", r"[i\-]", "-", true)]
#[case::leading_hyphen("regex", "[-i]", "-", true)]
#[case::hyphen_after_close_literal("regex", "[]i-]", "-", true)]
#[case::hyphen_dotted_case("regex", "[i-]", "\u{130}", true)]
#[case::comment_flags("regex", "(?x)# (?-i: \ni$", "\u{130}\n", true)]
#[case::named_scoped_flags("regex", "(?-i:(?P<id>a)i)$", "a\u{130}\n", false)]
#[case::named_condition_underscore("regex", "(?P<_id>a)(?(_id)i|x)$", "ai\n", true)]
#[case::numbered_condition("regex", "(a)(?(1)i|x)$", "ai\n", true)]
#[case::unmatched_condition("regex", "(?P<id>a)?(?(id)i|x)$", "x\n", true)]
#[case::scoped_reenable("regex", "(?-i:(?i:i))", "\u{130}", true)]
#[case::plain_hash("regex", "#i$", "#\u{130}\n", true)]
#[case::verbose_tab_class("regex", "(?x)[\t]$", "\t\n", true)]
#[case::scoped_verbose_restore("regex", "(?x:a # i $\n)#i$", "a#\u{130}\n", true)]
#[case::negated_non_i("regex", "[^x]", "x", false)]
#[case::nonboundary_separator("regex", r"a\B-", "a-", false)]
#[case::boundary_mark("regex", r"a\b", "a\u{301}", true)]
#[case::double_escape("regex", r"\\i$", "\\I\n", true)]
#[case::scoped_case_restored("regex", "(?-i:i)i$", "i\u{130}\n", true)]
#[case::scoped_case_sensitive("regex", "(?-i:i)i", "\u{130}i", false)]
#[case::named_group("regex", "(?P<id>i)(?P=id)$", "ii\n", true)]
#[case::conditional_group("regex", "(?P<id>a)(?(id)i|x)$", "ai\n", true)]
#[case::inline_comment("regex", "(?# i $)i$", "I\n", true)]
#[case::range_case("regex", "[h-j]", "\u{131}", true)]
#[case::negated_range("regex", "[^h-j]", "\u{131}", false)]
#[case::dotted_class("regex", "[\u{130}]", "i", true)]
#[case::dotless_class("regex", "[\u{131}]", "i", true)]
#[case::sensitive_class("regex", "(?-i:[i])", "\u{130}", false)]
#[case::word_class("regex", r"[\w]", "\u{301}", false)]
#[case::nonword("regex", r"\W", "\u{301}", true)]
#[case::nonboundary("regex", r"a\Bb", "ab", true)]
#[case::empty_nonboundary("regex", r"\B", "", false)]
#[case::class_backspace("regex", r"[\b]", "\x08", true)]
#[case::space_class("regex", r"[\s]", "\u{1c}", true)]
#[case::nonspace_class("regex", r"[\S]", "\u{1c}", false)]
#[case::verbose_class("regex", "(?x)[ #]$", "#\n", true)]
#[case::trailing_newline("regex", "secret$", "secret\n", true)]
#[case::two_trailing_newlines("regex", "secret$", "secret\n\n", false)]
#[case::strict_end("regex", r"secret\Z", "secret\n", false)]
#[case::multiline("regex", "(?m)secret$", "secret\nother", true)]
#[case::escaped_dollar("regex", r"secret\$", "secret$", true)]
#[case::dollar_class("regex", "secret[$]", "secret$", true)]
#[case::class_close_literal("regex", "[]$]", "$", true)]
#[case::comment_anchor("regex", "(?x)secret # $ (\n$", "secret\n", true)]
#[case::keyword_combining_mark("prebuilt", "nl_bsn_contextual", "BSN\u{301} 123456789", true)]
#[case::keyword_control_space(
    "prebuilt",
    "nl_bsn_contextual",
    "burger\u{1c}service nummer 123456789",
    true
)]
#[case::keyword_dotted_i(
    "prebuilt",
    "nl_bsn_contextual",
    "c\u{130}tizen service number 123456789",
    true
)]
#[case::keyword_dotless_i(
    "prebuilt",
    "nl_bsn_contextual",
    "c\u{131}tizen service number 123456789",
    true
)]
fn pattern_matching_behavior(
    #[case] kind: &str,
    #[case] source: &str,
    #[case] text: &str,
    #[case] expected: bool,
) {
    let entry = compile(&ContentFilterPattern {
        pattern_type: kind.to_owned(),
        pattern_name: (kind == "prebuilt").then(|| source.to_owned()),
        pattern: (kind == "regex").then(|| source.to_owned()),
        name: None,
        action: ContentFilterAction::Mask,
    })
    .unwrap();
    assert_eq!(!entry.find_spans(text).unwrap().is_empty(), expected);
}

#[path = "errors/tests.rs"]
mod errors;

#[test]
fn replays_the_content_regex_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/content_regex");
    let outcomes: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let source = fs::read_to_string(entry.unwrap().path()).unwrap();
            compile(&ContentFilterPattern {
                pattern_type: "regex".to_owned(),
                pattern_name: None,
                pattern: Some(source),
                name: None,
                action: ContentFilterAction::Mask,
            })
        })
        .collect();
    assert!(!outcomes.is_empty());
}
