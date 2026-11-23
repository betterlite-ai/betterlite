use rstest::rstest;

use crate::openapi_tools::naming::{TOOL_NAME_MAX_LEN, sanitize_openapi_tool_name};

#[rstest]
#[case::empty("", "")]
#[case::letters_and_digits("getUser2", "getuser2")]
#[case::slash_becomes_underscore("actions/download-job-logs", "actions_download-job-logs")]
#[case::dash_and_underscore_kept("list_pets-2", "list_pets-2")]
#[case::dots_and_spaces("get.pet name", "get_pet_name")]
#[case::unicode_becomes_one_underscore_each("caf\u{e9}s", "caf_s")]
#[case::already_lower("getpets", "getpets")]
fn maps_names_to_the_provider_character_class(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(sanitize_openapi_tool_name(raw), expected);
}

#[test]
fn keeps_exactly_128_characters() {
    let raw = "a".repeat(TOOL_NAME_MAX_LEN);
    assert_eq!(sanitize_openapi_tool_name(&raw), raw);
}

#[test]
fn cuts_a_longer_name_at_128_characters() {
    let raw = format!("{}{}", "a".repeat(TOOL_NAME_MAX_LEN), "ignored");
    let sanitized = sanitize_openapi_tool_name(&raw);
    assert_eq!(sanitized.len(), TOOL_NAME_MAX_LEN);
    assert_eq!(sanitized, "a".repeat(TOOL_NAME_MAX_LEN));
}

#[test]
fn cuts_before_lowercasing_the_tail() {
    let raw = format!("{}{}", "A".repeat(TOOL_NAME_MAX_LEN - 1), "Tail");
    assert_eq!(
        sanitize_openapi_tool_name(&raw),
        format!("{}t", "a".repeat(TOOL_NAME_MAX_LEN - 1))
    );
}
