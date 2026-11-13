#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{DEFAULT_BASE_URL, ModelscopeChatError, complete_url, flatten_messages, provider_info};
use crate::chat::ChatMessage;

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).unwrap()
}

fn flattened(text: &str) -> serde_json::Value {
    let mut parsed = messages(text);
    flatten_messages(&mut parsed).unwrap();
    serde_json::to_value(parsed).unwrap()
}

#[rstest]
#[case::default_base(None, "https://api-inference.modelscope.cn/v1/chat/completions")]
#[case::bare_base(Some("https://host.test/v1"), "https://host.test/v1/chat/completions")]
#[case::complete_url(
    Some("https://host.test/v1/chat/completions"),
    "https://host.test/v1/chat/completions"
)]
#[case::trailing_slash_is_not_trimmed(
    Some("https://host.test/v1/"),
    "https://host.test/v1//chat/completions"
)]
#[case::empty_base_is_the_default(
    Some(""),
    "https://api-inference.modelscope.cn/v1/chat/completions"
)]
fn the_url_appends_the_chat_path_unless_it_is_already_there(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(api_base), expected);
}

#[test]
fn the_default_base_is_the_model_scope_inference_endpoint() {
    assert_eq!(DEFAULT_BASE_URL, "https://api-inference.modelscope.cn/v1");
}

#[test]
fn the_environment_base_reaches_provider_info_but_not_the_url() {
    let (base, _) = provider_info(None, Some("https://env.test/v1"), None, None);
    assert_eq!(base, "https://env.test/v1");
    assert_eq!(
        complete_url(None),
        "https://api-inference.modelscope.cn/v1/chat/completions"
    );
}

#[test]
fn an_explicit_base_and_key_win_over_the_environment() {
    let explicit_key = key("explicit-key");
    let environment_key = key("env-key");
    let (base, chosen) = provider_info(
        Some("https://explicit.test/v1"),
        Some("https://env.test/v1"),
        Some(&explicit_key),
        Some(&environment_key),
    );
    assert_eq!(base, "https://explicit.test/v1");
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("explicit-key".to_owned())
    );
}

#[test]
fn the_environment_key_is_used_when_the_call_has_none() {
    let environment_key = key("env-key");
    let (_, chosen) = provider_info(None, None, None, Some(&environment_key));
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("env-key".to_owned())
    );
}

#[test]
fn an_empty_key_counts_as_unset_at_every_step() {
    let explicit_key = key("");
    let environment_key = key("");
    let (_, chosen) = provider_info(None, None, Some(&explicit_key), Some(&environment_key));
    assert!(chosen.is_none());
}

#[test]
fn a_text_only_list_becomes_the_parts_joined_with_nothing() {
    let input = r#"[{"role": "user", "content": [{"type": "text", "text": "Hello "}, {"type": "text", "text": "world"}]}]"#;
    let expected = serde_json::json!([{"role": "user", "content": "Hello world"}]);
    assert_eq!(flattened(input), expected);
}

#[test]
fn a_list_with_one_non_text_part_keeps_its_list_whole() {
    let input = r#"[{"role": "user", "content": [{"type": "text", "text": "look"}, {"type": "image_url", "image_url": {"url": "https://host.test/a.png"}}]}]"#;
    assert_eq!(
        flattened(input),
        serde_json::json!([{"role": "user", "content": [
            {"type": "text", "text": "look"},
            {"type": "image_url", "image_url": {"url": "https://host.test/a.png"}}
        ]}])
    );
}

#[test]
fn a_non_text_part_first_still_keeps_the_list() {
    let input = r#"[{"role": "user", "content": [{"type": "input_audio", "input_audio": {"data": "AA", "format": "wav"}}, {"type": "text", "text": "hear"}]}]"#;
    assert_eq!(
        flattened(input),
        serde_json::json!([{"role": "user", "content": [
            {"type": "input_audio", "input_audio": {"data": "AA", "format": "wav"}},
            {"type": "text", "text": "hear"}
        ]}])
    );
}

#[rstest]
#[case::plain_text(r#"[{"role": "user", "content": "as sent"}]"#, serde_json::json!([{"role": "user", "content": "as sent"}]))]
#[case::absent_content(r#"[{"role": "assistant", "tool_calls": [{"id": "call", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}]"#, serde_json::json!([{"role": "assistant", "tool_calls": [{"id": "call", "type": "function", "function": {"name": "f", "arguments": "{}"}}]}]))]
fn a_message_without_a_part_list_is_untouched(
    #[case] input: &str,
    #[case] expected: serde_json::Value,
) {
    assert_eq!(flattened(input), expected);
}

#[rstest]
#[case::empty_list(
    r#"[{"role": "user", "content": []}]"#,
    serde_json::json!([{"role": "user", "content": ""}])
)]
#[case::missing_text(
    r#"[{"role": "user", "content": [{"type": "text"}, {"type": "text", "text": "b"}]}]"#,
    serde_json::json!([{"role": "user", "content": "b"}])
)]
#[case::assistant_role(
    r#"[{"role": "assistant", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}]"#,
    serde_json::json!([{"role": "assistant", "content": "ab"}])
)]
fn the_join_keeps_every_text_and_skips_a_part_without_one(
    #[case] input: &str,
    #[case] expected: serde_json::Value,
) {
    assert_eq!(flattened(input), expected);
}

#[test]
fn each_message_is_judged_on_its_own() {
    let input = r#"[
        {"role": "system", "content": [{"type": "text", "text": "sys"}]},
        {"role": "user", "content": [{"type": "text", "text": "a"}, {"type": "image_url", "image_url": {"url": "https://host.test/a.png"}}]},
        {"role": "user", "content": "plain"}
    ]"#;
    assert_eq!(
        flattened(input),
        serde_json::json!([
            {"role": "system", "content": "sys"},
            {"role": "user", "content": [
                {"type": "text", "text": "a"},
                {"type": "image_url", "image_url": {"url": "https://host.test/a.png"}}
            ]},
            {"role": "user", "content": "plain"}
        ])
    );
}

#[test]
fn a_text_part_whose_text_is_not_a_string_names_the_message() {
    let mut parsed = messages(
        r#"[
        {"role": "user", "content": "plain"},
        {"role": "user", "content": [{"type": "text", "text": 5}]}
    ]"#,
    );
    assert!(matches!(
        flatten_messages(&mut parsed),
        Err(ModelscopeChatError::TextNotString { index: 1 })
    ));
}

#[test]
fn a_null_text_contributes_nothing_like_an_absent_one() {
    let input = r#"[{"role": "user", "content": [{"type": "text", "text": null}, {"type": "text", "text": "b"}]}]"#;
    assert_eq!(
        flattened(input),
        serde_json::json!([{"role": "user", "content": "b"}])
    );
}
