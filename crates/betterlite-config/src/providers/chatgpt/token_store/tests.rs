#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::resolve;

const API_BASE: &str = "https://chatgpt.com/backend-api/codex";
const PRIMARY: &str = "https://primary.example";
const SECONDARY: &str = "https://secondary.example";
const TOKEN_DIR: &str = "~/.config/litellm/chatgpt";

#[rstest]
#[case::defaults(&[], (TOKEN_DIR, "auth.json", API_BASE))]
#[case::home_expands_default(&[("HOME", "/home/alice")], ("/home/alice/.config/litellm/chatgpt", "auth.json", API_BASE))]
#[case::explicit_dir(&[("CHATGPT_TOKEN_DIR", "/custom")], ("/custom", "auth.json", API_BASE))]
#[case::explicit_empty_dir(&[("CHATGPT_TOKEN_DIR", "")], ("", "auth.json", API_BASE))]
#[case::explicit_file(&[("CHATGPT_AUTH_FILE", "other.json")], (TOKEN_DIR, "other.json", API_BASE))]
#[case::explicit_empty_file(&[("CHATGPT_AUTH_FILE", "")], (TOKEN_DIR, "", API_BASE))]
#[case::primary_api_base(&[("CHATGPT_API_BASE", PRIMARY)], (TOKEN_DIR, "auth.json", PRIMARY))]
#[case::secondary_api_base(&[("OPENAI_CHATGPT_API_BASE", SECONDARY)], (TOKEN_DIR, "auth.json", SECONDARY))]
#[case::empty_primary_falls_through(&[("CHATGPT_API_BASE", ""), ("OPENAI_CHATGPT_API_BASE", SECONDARY)], (TOKEN_DIR, "auth.json", SECONDARY))]
fn chatgpt_token_paths_load(#[case] env: &[(&str, &str)], #[case] expected: (&str, &str, &str)) {
    let lookup: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let resolved = resolve(&lookup);
    assert_eq!(resolved.token_dir, expected.0);
    assert_eq!(resolved.auth_file, expected.1);
    assert_eq!(resolved.api_base, expected.2);
}
