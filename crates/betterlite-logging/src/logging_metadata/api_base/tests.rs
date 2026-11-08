#![cfg(test)]

use rstest::rstest;

use super::strip_trailing_slash;

#[rstest]
#[case::absent(None, None)]
#[case::no_slash(Some("https://api.example.com"), Some("https://api.example.com"))]
#[case::one_slash(Some("https://api.example.com/"), Some("https://api.example.com"))]
#[case::two_slashes(Some("https://api.example.com//"), Some("https://api.example.com"))]
#[case::many_slashes(Some("https://api.example.com///"), Some("https://api.example.com"))]
#[case::only_slashes(Some("///"), Some(""))]
#[case::empty(Some(""), Some(""))]
#[case::inner_slashes(
    Some("https://api.example.com/v1/"),
    Some("https://api.example.com/v1")
)]
fn logging_metadata_behavior_tests_behavior(
    #[case] api_base: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(strip_trailing_slash(api_base).as_deref(), expected);
}
