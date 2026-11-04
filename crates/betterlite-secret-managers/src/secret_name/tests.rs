#![cfg(test)]

use super::raise_if_unsafe_secret_name;
use rstest::rstest;

#[test]
fn accepts_safe_names_and_double_dots_inside_segments() {
    assert_eq!(raise_if_unsafe_secret_name("release-1.0..2"), Ok(()));
    assert_eq!(raise_if_unsafe_secret_name("nested/name"), Ok(()));
}

#[rstest]
#[case::exact_parent("..")]
#[case::parent_prefix("../name")]
#[case::parent_suffix("name/..")]
#[case::parent_middle("name/../other")]
fn rejects_parent_path_segments(#[case] name: &str) {
    assert!(raise_if_unsafe_secret_name(name).is_err());
}

#[rstest]
#[case::nul('\u{0000}')]
#[case::unit_separator('\u{001f}')]
#[case::delete('\u{007f}')]
#[case::c1_first('\u{0080}')]
#[case::c1_last('\u{009f}')]
#[case::space(' ')]
#[case::tab('\t')]
#[case::line_separator('\u{2028}')]
#[case::paragraph_separator('\u{2029}')]
fn rejects_controls_spaces_and_tabs(#[case] character: char) {
    assert!(raise_if_unsafe_secret_name(&format!("name{character}part")).is_err());
}
