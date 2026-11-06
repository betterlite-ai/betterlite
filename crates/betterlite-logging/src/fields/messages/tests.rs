#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::append_system_prompt_messages;

fn decode(encoded: &str) -> Json {
    serde_json::from_str(encoded).unwrap()
}

#[rstest]
#[case::no_system(
    None,
    Some(r#"{"role":"user","content":"hi"}"#),
    r#"{"role":"user","content":"hi"}"#
)]
#[case::system_with_no_messages(
    Some("be nice"),
    None,
    r#"[{"role":"system","content":"be nice"}]"#
)]
#[case::system_with_empty_list(
    Some("be nice"),
    Some("[]"),
    r#"[{"role":"system","content":"be nice"}]"#
)]
#[case::system_already_first(
    Some("be nice"),
    Some(r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#),
    r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#
)]
#[case::different_system_is_prepended(
    Some("be nice"),
    Some(r#"[{"role":"system","content":"other"}]"#),
    r#"[{"role":"system","content":"be nice"},{"role":"system","content":"other"}]"#
)]
#[case::user_message_is_prepended(
    Some("be nice"),
    Some(r#"[{"role":"user","content":"hi"}]"#),
    r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#
)]
#[case::text_becomes_a_user_message(
    Some("be nice"),
    Some(r#""hi""#),
    r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#
)]
#[case::a_mapping_passes_through(Some("be nice"), Some(r#"{"role":"user"}"#), r#"{"role":"user"}"#)]
#[case::a_number_passes_through(Some("be nice"), Some("7"), "7")]
#[case::empty_system_text_is_still_added(
    Some(""),
    Some(r#"[{"role":"user","content":"hi"}]"#),
    r#"[{"role":"system","content":""},{"role":"user","content":"hi"}]"#
)]
fn logging_fields_behavior_tests_behavior(
    #[case] system: Option<&str>,
    #[case] messages: Option<&str>,
    #[case] expected: &str,
) {
    let parsed_messages = messages.map(decode);
    assert_eq!(
        append_system_prompt_messages(system, parsed_messages.as_ref()),
        Some(decode(expected))
    );
}

#[test]
fn a_non_string_system_prompt_leaves_the_messages_alone() {
    let messages = decode(r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(
        append_system_prompt_messages(None, Some(&messages)),
        Some(messages)
    );
}
