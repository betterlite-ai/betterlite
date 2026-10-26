#![cfg(test)]
use rstest::rstest;

use crate::sandbox_backend::replace::{ReplaceError, Replaced, replace_occurrences};

#[rstest]
#[case::single("a b c", false, "a x c", 1)]
#[case::all("b b c", true, "x x c", 2)]
#[case::all_with_one_match("a b c", true, "a x c", 1)]
fn replaces_and_counts_occurrences(
    #[case] content: &str,
    #[case] replace_all: bool,
    #[case] expected: &str,
    #[case] occurrences: usize,
) {
    assert_eq!(
        replace_occurrences(content, "b", "x", replace_all),
        Ok(Replaced {
            content: expected.to_owned(),
            occurrences,
        })
    );
}

#[rstest]
#[case::empty_old(
    "abc",
    "",
    "Error: old_string cannot be empty. Provide the exact text to replace."
)]
#[case::not_found("abc", "zz", "Error: String not found in file: 'zz'")]
#[case::ambiguous(
    "b b",
    "b",
    "Error: String 'b' appears 2 times in file. Use replace_all=True to replace all instances, or provide a more specific string with surrounding context."
)]
#[case::trailing_newline(
    "x\nlast",
    "last\n",
    "Error: old_string ends with a newline, but the file does not end with a newline. Retry with the trailing newline removed from old_string (and from new_string if it also ends with a newline)."
)]
#[case::trailing_newline_ambiguous(
    "last last",
    "last\n",
    "Error: old_string ends with a newline, but the file does not end with a newline. With the trailing newline removed, old_string would appear 2 times in the file. Retry with the trailing newline removed and add surrounding context so the match is unique."
)]
#[case::lone_newline_is_not_found("abc", "\n", "Error: String not found in file: '\n'")]
fn explains_why_nothing_was_replaced(
    #[case] content: &str,
    #[case] old: &str,
    #[case] message: &str,
) {
    let error = replace_occurrences(content, old, "new", false).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn refuses_several_matches_without_replace_all() {
    assert_eq!(
        replace_occurrences("b b b", "b", "x", false),
        Err(ReplaceError::Ambiguous {
            old: "b".to_owned(),
            count: 3,
        })
    );
}
