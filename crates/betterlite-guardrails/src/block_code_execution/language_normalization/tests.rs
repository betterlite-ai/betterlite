#![cfg(test)]

use rstest::rstest;

use super::{LanguageMatch, confidence_for_block, match_language, normalize_language};

#[rstest]
#[case::js_alias("js", "javascript")]
#[case::py_alias_uppercase("PY", "python")]
#[case::sh_alias_padded(" Sh ", "bash")]
#[case::ts_alias("ts", "typescript")]
#[case::plain_tag_lowercased("Rust", "rust")]
#[case::unicode_lowercased("\u{c9}LAN", "\u{e9}lan")]
fn normalizes_case_and_aliases(#[case] tag: &str, #[case] expected: &str) {
    assert_eq!(normalize_language(tag), expected);
}

#[rstest]
#[case::empty_list_blocks_all("rust", &[], LanguageMatch::BlockAll)]
#[case::alias_on_the_tag("js", &["JavaScript"], LanguageMatch::Listed)]
#[case::alias_on_the_list("python", &["py"], LanguageMatch::Listed)]
#[case::case_insensitive("BASH", &["sh"], LanguageMatch::Listed)]
#[case::untagged_not_listed("", &["python"], LanguageMatch::Unlisted)]
#[case::other_language("rust", &["python", "bash"], LanguageMatch::Unlisted)]
fn matches_tags_against_the_blocked_list(
    #[case] tag: &str,
    #[case] blocked: &[&str],
    #[case] expected: LanguageMatch,
) {
    let blocked_languages: Vec<String> = blocked
        .iter()
        .map(|language| (*language).to_owned())
        .collect();
    assert_eq!(match_language(tag, &blocked_languages), expected);
}

#[rstest]
#[case::listed_tag("python", LanguageMatch::Listed, 1.0_f64)]
#[case::listed_non_executable_tag("text", LanguageMatch::Listed, 1.0_f64)]
#[case::block_all_text("text", LanguageMatch::BlockAll, 0.5_f64)]
#[case::block_all_markdown_uppercase("MD", LanguageMatch::BlockAll, 0.5_f64)]
#[case::block_all_output("output", LanguageMatch::BlockAll, 0.5_f64)]
#[case::block_all_executable("python", LanguageMatch::BlockAll, 1.0_f64)]
#[case::block_all_untagged("", LanguageMatch::BlockAll, 1.0_f64)]
#[case::unlisted("rust", LanguageMatch::Unlisted, 0.0_f64)]
fn scores_confidence(
    #[case] tag: &str,
    #[case] language_match: LanguageMatch,
    #[case] expected: f64,
) {
    assert_eq!(
        confidence_for_block(tag, language_match).to_bits(),
        expected.to_bits()
    );
}
