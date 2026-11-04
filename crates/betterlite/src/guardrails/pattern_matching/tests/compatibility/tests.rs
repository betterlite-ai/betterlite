#![cfg(test)]

use std::fs;
use std::path::Path;

use super::{ContentFilterAction, ContentFilterPattern, compile};
use rstest::rstest;

#[rstest]
#[case::nested_sensitive_unicode("regex", "(?-i:(?u:a))", "A", false)]
#[case::nested_sensitive_reference("regex", r"(?a-i:(a)(?a:\1))", "aA", false)]
#[case::ascii_word_class_stays_narrow("regex", "(?a:(\u{e9})\\1[\\w])", "\u{e9}\u{c9}a", false)]
#[case::ascii_digit_class_stays_narrow("regex", "(?a:(\u{e9})\\1[\\d])", "\u{e9}\u{c9}1", false)]
#[case::ascii_space_class_stays_narrow("regex", "(?a:(\u{e9})\\1[\\s])", "\u{e9}\u{c9} ", false)]
#[case::sensitive_word_class_is_wide("regex", r"(?a:(a)\1)(?-i:[\w])", "aa\u{e9}", true)]
#[case::sensitive_digit_class_is_wide("regex", r"(?a:(a)\1)(?-i:[\d])", "aa\u{661}", true)]
#[case::sensitive_space_class_is_wide("regex", r"(?a:(a)\1)(?-i:[\s])", "aa\u{1c}", true)]
#[case::mixed_ascii_class("regex", r"(?a:(a)\1)[k]", "aa\u{212a}", true)]
#[case::sensitive_narrow_class("regex", "(?a:(\u{e9})\\1)(?-i:[a])", "\u{e9}\u{c9}a", false)]
#[case::ascii_narrow_class("regex", "(?a:(\u{e9})\\1[a])", "\u{e9}\u{c9}a", false)]
#[case::ascii_wide_literal_class("regex", "(?a:[\u{e9}](a)\\1)", "\u{e9}aa", true)]
#[case::ascii_wide_escaped_class("regex", r"(?a:[\u00e9](a)\1)", "\u{e9}aa", true)]
#[case::sensitive_class_matches("regex", "(?a:(a)\\1)(?-i:[\u{e9}])", "aa\u{e9}", true)]
#[case::sensitive_literal_keeps_ascii(
    "regex",
    "(?a:(\u{e9})\\1)(?-i:\u{e9})",
    "\u{e9}\u{c9}\u{e9}",
    false
)]
#[case::sensitive_escape_keeps_ascii(
    "regex",
    "(?a:(\u{e9})\\1)(?-i:\\u00e9)",
    "\u{e9}\u{c9}\u{e9}",
    false
)]
#[case::sensitive_wide_class("regex", "(?a:(a)\\1)(?-i:[\u{e9}])", "aa\u{c9}", false)]
#[case::sensitive_reference_keeps_ascii(
    "regex",
    "(?a:(\u{e9})\\1)(?-i:\\1)",
    "\u{e9}\u{c9}\u{e9}",
    false
)]
#[case::sensitive_named_keeps_ascii(
    "regex",
    "(?a:(?P<x>\u{e9})(?P=x))(?-i:(?P=x))",
    "\u{e9}\u{c9}\u{e9}",
    false
)]
#[case::alphabet_keeps_case_off("regex", "(?-i:(?a:a))", "A", false)]
#[case::mixed_literal_after("regex", "(?a:(a)\\1)\u{e9}", "aa\u{c9}", true)]
#[case::mixed_literal_before("regex", "\u{e9}(?a:(a)\\1)", "\u{c9}aa", true)]
#[case::mixed_ascii_literal("regex", r"(?a:(a)\1)k", "aa\u{212a}", true)]
#[case::mixed_escaped_literal("regex", r"(?a:(a)\1)\u00e9", "aa\u{c9}", true)]
#[case::mixed_unicode_class("regex", "(?a:(a)\\1)[\u{e9}]", "aa\u{c9}", true)]
#[case::mixed_escaped_class("regex", r"(?a:(a)\1)[\u00e9]", "aa\u{c9}", true)]
#[case::mixed_unicode_reference("regex", "(?a:(a)\\1)(?u:(\u{e9})\\2)", "aa\u{e9}\u{c9}", true)]
#[case::mixed_inherited_reference("regex", "(?a:(a)\\1)(\u{e9})\\2", "aa\u{e9}\u{c9}", true)]
#[case::mixed_reference_to_ascii("regex", "(?a:(\u{e9})\\1)\\1", "\u{e9}\u{e9}\u{c9}", true)]
#[case::mixed_named_reference(
    "regex",
    "(?a:(?P<x>\u{e9})(?P=x))(?u:(?P=x))",
    "\u{e9}\u{e9}\u{c9}",
    true
)]
#[case::mixed_sensitive_literal("regex", "(?a:(a)\\1)(?-i:\u{e9})", "aa\u{c9}", false)]
#[case::mixed_sensitive_reference(
    "regex",
    "(?a:(\u{e9})\\1)(?-i:\\1)",
    "\u{e9}\u{e9}\u{c9}",
    false
)]
#[case::ascii_reference_nonword("regex", r"(?a:(a)\1\W)", "aa\u{e9}", true)]
#[case::ascii_reference_nondigit("regex", r"(?a:(a)\1\D)", "aa\u{e9}", true)]
#[case::ascii_reference_nonspace("regex", r"(?a:(a)\1\S)", "aa\u{e9}", true)]
#[case::ascii_reference_class_nonword("regex", r"(?a:(a)\1[\W])", "aa\u{e9}", true)]
#[case::ascii_reference_class_nondigit("regex", r"(?a:(a)\1[\D])", "aa\u{e9}", true)]
#[case::ascii_reference_class_nonspace("regex", r"(?a:(a)\1[\S])", "aa\u{e9}", true)]
#[case::ascii_nonword_excludes_word("regex", r"(?a:(a)\1\W)", "aa_", false)]
#[case::ascii_nondigit_excludes_digit("regex", r"(?a:(a)\1[\D])", "aa1", false)]
#[case::ascii_nonspace_excludes_space("regex", r"(?a:(a)\1\S)", "aa ", false)]
#[case::ascii_global("regex", r"(?a)\w", "\u{e9}", false)]
#[case::ascii_boundary("regex", r"(?a:a\b)", "a\u{e9}", true)]
#[case::ascii_nonboundary("regex", r"(?a:a\B)", "a\u{e9}", false)]
#[case::ascii_space("regex", r"(?a:\s)", "\u{1c}", false)]
#[case::ascii_space_matches_literal_space("regex", r"(?a:\s)", " ", true)]
#[case::ascii_nonspace("regex", r"(?a:\S)", "\u{1c}", true)]
#[case::ascii_digit("regex", r"(?a:\d)", "\u{661}", false)]
#[case::ascii_nondigit("regex", r"(?a:\D)", "\u{661}", true)]
#[case::ascii_disabled_case("regex", "(?a-i:i)", "I", false)]
#[case::ascii_upper_class("regex", "(?a:[A])", "a", true)]
#[case::ascii_class_nonmember("regex", "(?a:[A])", "b", false)]
#[case::ascii_class_no_unicode_fold("regex", "(?a:[\u{212a}])", "k", false)]
#[case::ascii_negated_class("regex", "(?a:[^a-z])", "\u{130}", true)]
#[case::case_disabled_range("regex", "(?-i:[a-z])", "A", false)]
#[case::unicode_digit("regex", r"\d", "\u{661}", true)]
#[case::octal_zero("regex", r"\0", "\0", true)]
#[case::octal_two_digits("regex", r"\01", "\x01", true)]
#[case::octal_maximum("regex", r"\377", "\u{ff}", true)]
#[case::class_short_octal("regex", r"[\1]", "\x01", true)]
#[case::octal_followed_digit("regex", r"\0123", "\n3", true)]
#[case::two_digit_backreference(
    "regex",
    r"(a)(b)(c)(d)(e)(f)(g)(h)(i)(j)(k)(l)\12",
    "abcdefghijkll",
    true
)]
#[case::escaped_bell("regex", r"\a", "\x07", true)]
#[case::escaped_formfeed("regex", r"\f", "\x0c", true)]
#[case::escaped_return("regex", r"\r", "\r", true)]
#[case::escaped_tab("regex", r"\t", "\t", true)]
#[case::escaped_vertical_tab("regex", r"\v", "\x0b", true)]
#[case::ascii_escaped_i("regex", r"(?a:\x69)", "\u{130}", false)]
#[case::escaped_metacharacter("regex", r"\x2e", "a", false)]
#[case::hex_control("regex", r"\x1c", "\u{1c}", true)]
#[case::ascii_scope_keeps_verbose_off("regex", "(?a)#\\s\n", "#\u{2003}\n", false)]
#[case::unicode_flags("regex", "(?ui)secret", "SECRET", true)]
#[case::ascii_scope("regex", r"(?a:\w+)", "SECRET", true)]
#[case::ascii_word_excludes_unicode("regex", r"(?a:\w+)", "\u{e9}", false)]
#[case::ascii_case_excludes_dotted("regex", "(?ai:i)", "\u{130}", false)]
#[case::ascii_case_excludes_kelvin("regex", "(?a:k)", "\u{212a}", false)]
#[case::ascii_nonascii_literal("regex", "(?a:\u{e9})", "\u{c9}", false)]
#[case::unicode_scope_restores("regex", r"(?a:(?u:\w))", "\u{e9}", true)]
#[case::ascii_scope_restores("regex", r"(?a:a)\w", "a\u{e9}", true)]
#[case::ascii_class_case("regex", "(?a:[a-z])", "Z", true)]
#[case::ascii_class_no_unicode("regex", "(?a:[a-z])", "\u{130}", false)]
#[case::octal_literal("regex", r"\141", "a", true)]
#[case::octal_i("regex", r"\151", "\u{130}", true)]
#[case::hex_i("regex", r"\x69", "\u{130}", true)]
#[case::unicode_i("regex", r"\u0069", "\u{130}", true)]
#[case::long_unicode_i("regex", r"\U00000069", "\u{130}", true)]
#[case::class_octal("regex", r"[\141]", "A", true)]
#[case::class_hex_i("regex", r"[\x69]", "\u{130}", true)]
#[case::numeric_backreference("regex", r"(a)\1", "aa", true)]
#[case::ascii_backreference_folds_ascii("regex", r"(?a:(a)\1)", "aA", true)]
#[case::ascii_backreference_excludes_unicode_fold(
    "regex",
    "(?a:(\u{e9})\\1)",
    "\u{e9}\u{c9}",
    false
)]
#[case::ascii_named_backreference_excludes_unicode_fold(
    "regex",
    "(?a:(?P<id>\u{e9})(?P=id))",
    "\u{e9}\u{c9}",
    false
)]
#[case::wide_escape_keeps_backreference_folded(
    "regex",
    "\\w(?a:(\u{e9})\\1)",
    "x\u{e9}\u{c9}",
    true
)]
#[case::wide_class_escape_keeps_backreference_folded(
    "regex",
    "[\\w](?a:(\u{e9})\\1)",
    "x\u{e9}\u{c9}",
    true
)]
#[case::wide_dot_keeps_backreference_folded("regex", ".(?a:(\u{e9})\\1)", "x\u{e9}\u{c9}", true)]
#[case::class_literal_open("regex", "[[]", "[", true)]
#[case::word_not_folded("regex", r"\w", "\u{345}", false)]
#[case::nonword_not_folded("regex", r"\W", "\u{345}", true)]
#[case::class_word_not_folded("regex", r"[\w]", "\u{345}", false)]
#[case::class_nonword_not_folded("regex", r"[\W]", "\u{345}", true)]
#[case::mixed_word_class("regex", r"[\wi]", "\u{130}", true)]
#[case::mixed_word_class_excludes_mark("regex", r"[\wi]", "\u{345}", false)]
#[case::negated_word_class("regex", r"[^\w]", "\u{345}", true)]
#[case::word_numbers_dotted_prefix(
    "prebuilt",
    "nl_bsn_contextual",
    "BSN \u{130}one two three four five six seven eight nine",
    false
)]
#[case::word_numbers_dotted_suffix(
    "prebuilt",
    "nl_bsn_contextual",
    "BSN one two three four five six seven eight nine\u{130}",
    false
)]
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
