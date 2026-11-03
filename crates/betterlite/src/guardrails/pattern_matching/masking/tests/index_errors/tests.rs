#![cfg(test)]

use super::redaction_tag;
use rstest::rstest;

#[rstest]
#[case::signed_index("{pattern_name[+0]}", "email")]
#[case::out_of_range("{pattern_name[5]}", "email")]
#[case::empty_name("{pattern_name[0]}", "")]
#[case::negative_index("{pattern_name[-1]}", "email")]
#[case::nonnumeric_index("{pattern_name[x]}", "email")]
#[case::empty_index("{pattern_name[]}", "email")]
#[case::missing_close("{pattern_name[0}", "email")]
#[case::unexpected_suffix("{pattern_name[0]x}", "email")]
#[case::nested_out_of_range("{pattern_name[0][1]}", "email")]
#[case::index_overflow("{pattern_name[999999999999999999999999999999999]}", "email")]
fn pattern_matching_behavior(#[case] format: &str, #[case] name: &str) {
    redaction_tag(format, name).unwrap_err();
}
