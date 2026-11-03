#![cfg(test)]

use super::{ContentFilterAction, ContentFilterPattern, compile};
use rstest::rstest;

#[rstest]
#[case::missing_name("prebuilt", None, "pattern_name is required for prebuilt patterns")]
#[case::missing_regex("regex", None, "pattern is required for regex patterns")]
#[case::unknown_type("other", None, "Unknown pattern_type: other")]
fn pattern_matching_behavior(
    #[case] kind: &str,
    #[case] regex: Option<&str>,
    #[case] message: &str,
) {
    let result = compile(&ContentFilterPattern {
        pattern_type: kind.to_owned(),
        pattern_name: None,
        pattern: regex.map(str::to_owned),
        name: None,
        action: ContentFilterAction::Block,
    });
    assert_eq!(result.unwrap_err().to_string(), message);
}
