#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::resolve;

fn lookup(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::defaults(
    &[],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::home_expands_default(
    &[("HOME", "/home/alice")],
    "/home/alice/.config/litellm/chatgpt",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_dir(
    &[("CHATGPT_TOKEN_DIR", "/custom")],
    "/custom",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_empty_dir(
    &[("CHATGPT_TOKEN_DIR", "")],
    "",
    "auth.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_file(
    &[("CHATGPT_AUTH_FILE", "other.json")],
    "~/.config/litellm/chatgpt",
    "other.json",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::explicit_empty_file(
    &[("CHATGPT_AUTH_FILE", "")],
    "~/.config/litellm/chatgpt",
    "",
    "https://chatgpt.com/backend-api/codex"
)]
#[case::primary_api_base(
    &[("CHATGPT_API_BASE", "https://primary.example")],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://primary.example"
)]
#[case::secondary_api_base(
    &[("OPENAI_CHATGPT_API_BASE", "https://secondary.example")],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://secondary.example"
)]
#[case::empty_primary_falls_through(
    &[
        ("CHATGPT_API_BASE", ""),
        ("OPENAI_CHATGPT_API_BASE", "https://secondary.example"),
    ],
    "~/.config/litellm/chatgpt",
    "auth.json",
    "https://secondary.example"
)]
fn chatgpt_token_paths_load(
    #[case] env: &[(&str, &str)],
    #[case] token_dir: &str,
    #[case] auth_file: &str,
    #[case] api_base: &str,
) {
    let resolved = resolve(&lookup(env));
    assert_eq!(resolved.token_dir, token_dir);
    assert_eq!(resolved.auth_file, auth_file);
    assert_eq!(resolved.api_base, api_base);
}
