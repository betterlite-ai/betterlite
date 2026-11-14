#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{Agent, printable, resolve_credentials};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::auth_token_beats_api_key(
    &[("ANTHROPIC_BASE_URL", "http://proxy"), ("ANTHROPIC_AUTH_TOKEN", "token"), ("ANTHROPIC_API_KEY", "key")],
    "token"
)]
#[case::api_key_when_token_is_empty(
    &[("ANTHROPIC_BASE_URL", "http://proxy"), ("ANTHROPIC_AUTH_TOKEN", ""), ("ANTHROPIC_API_KEY", "key")],
    "key"
)]
#[case::api_key_when_token_is_absent(
    &[("ANTHROPIC_BASE_URL", "http://proxy"), ("ANTHROPIC_API_KEY", "key")],
    "key"
)]
#[case::key_is_trimmed(
    &[("ANTHROPIC_BASE_URL", "http://proxy"), ("ANTHROPIC_AUTH_TOKEN", "  token \n")],
    "token"
)]
fn claude_code_key_resolution(#[case] pairs: &[(&str, &str)], #[case] expected: &str) {
    let credentials = resolve_credentials(&environment(pairs), Agent::ClaudeCode).unwrap();

    assert_eq!(credentials.api_key.expose_secret(), expected);
    assert_eq!(credentials.base_url, "http://proxy");
}

#[rstest]
#[case::plain("http://proxy:4000", "http://proxy:4000")]
#[case::trailing_slash("http://proxy:4000/", "http://proxy:4000")]
#[case::many_trailing_slashes("http://proxy:4000///", "http://proxy:4000")]
#[case::version_suffix("http://proxy:4000/v1", "http://proxy:4000")]
#[case::version_suffix_and_slash("http://proxy:4000/v1/", "http://proxy:4000")]
#[case::only_one_version_suffix("http://proxy/v1/v1", "http://proxy/v1")]
#[case::surrounding_whitespace("  http://proxy:4000/v1 \t", "http://proxy:4000")]
#[case::version_in_the_middle("http://proxy/v1/api", "http://proxy/v1/api")]
#[case::longer_version("http://proxy/v10", "http://proxy/v10")]
fn base_url_is_trimmed(#[case] raw: &str, #[case] expected: &str) {
    let claude_variables =
        environment(&[("ANTHROPIC_BASE_URL", raw), ("ANTHROPIC_API_KEY", "key")]);
    let claude = resolve_credentials(&claude_variables, Agent::ClaudeCode).unwrap();
    let codex_variables = environment(&[("OPENAI_BASE_URL", raw), ("OPENAI_API_KEY", "key")]);
    let codex = resolve_credentials(&codex_variables, Agent::Codex).unwrap();

    assert_eq!(claude.base_url, expected);
    assert_eq!(codex.base_url, expected);
}

#[test]
fn codex_reads_only_the_openai_variables() {
    let variables = environment(&[
        ("OPENAI_BASE_URL", "http://codex-proxy/v1"),
        ("OPENAI_API_KEY", "codex-key"),
        ("ANTHROPIC_BASE_URL", "http://claude-proxy"),
        ("ANTHROPIC_AUTH_TOKEN", "claude-token"),
    ]);

    let codex = resolve_credentials(&variables, Agent::Codex).unwrap();
    let claude = resolve_credentials(&variables, Agent::ClaudeCode).unwrap();

    assert_eq!(codex.base_url, "http://codex-proxy");
    assert_eq!(codex.api_key.expose_secret(), "codex-key");
    assert_eq!(claude.base_url, "http://claude-proxy");
    assert_eq!(claude.api_key.expose_secret(), "claude-token");
}

#[test]
fn claude_code_ignores_litellm_and_openai_variables() {
    let variables = environment(&[
        ("ANTHROPIC_BASE_URL", "http://proxy"),
        ("LITELLM_PROXY_API_KEY", "other"),
        ("OPENAI_API_KEY", "other"),
    ]);

    assert!(resolve_credentials(&variables, Agent::ClaudeCode).is_none());
}

#[rstest]
#[case::nothing(&[])]
#[case::no_base_url(&[("ANTHROPIC_API_KEY", "key")])]
#[case::no_key(&[("ANTHROPIC_BASE_URL", "http://proxy")])]
#[case::empty_base_url(&[("ANTHROPIC_BASE_URL", ""), ("ANTHROPIC_API_KEY", "key")])]
#[case::base_url_of_only_slashes(&[("ANTHROPIC_BASE_URL", "/"), ("ANTHROPIC_API_KEY", "key")])]
#[case::base_url_of_only_a_version(&[("ANTHROPIC_BASE_URL", "/v1"), ("ANTHROPIC_API_KEY", "key")])]
#[case::blank_key(&[("ANTHROPIC_BASE_URL", "http://proxy"), ("ANTHROPIC_API_KEY", "   ")])]
#[case::blank_token_wins_over_a_real_key(
    &[("ANTHROPIC_BASE_URL", "http://proxy"), ("ANTHROPIC_AUTH_TOKEN", " "), ("ANTHROPIC_API_KEY", "key")]
)]
fn unusable_credentials_resolve_to_nothing(#[case] pairs: &[(&str, &str)]) {
    assert!(resolve_credentials(&environment(pairs), Agent::ClaudeCode).is_none());
}

#[rstest]
#[case::codex_without_key(&[("OPENAI_BASE_URL", "http://proxy")])]
#[case::codex_without_base_url(&[("OPENAI_API_KEY", "key")])]
fn unusable_codex_credentials_resolve_to_nothing(#[case] pairs: &[(&str, &str)]) {
    assert!(resolve_credentials(&environment(pairs), Agent::Codex).is_none());
}

#[test]
fn credentials_debug_output_hides_the_key() {
    let variables = environment(&[
        ("ANTHROPIC_BASE_URL", "http://proxy"),
        ("ANTHROPIC_API_KEY", "sk-super-secret"),
    ]);

    let credentials = resolve_credentials(&variables, Agent::ClaudeCode).unwrap();

    assert!(!format!("{credentials:?}").contains("sk-super-secret"));
}

#[rstest]
#[case::plain("claude-sonnet-4", "claude-sonnet-4")]
#[case::keeps_inner_spaces("Claude Sonnet 4", "Claude Sonnet 4")]
#[case::escape("a\u{1b}[31mb", "a[31mb")]
#[case::bell("a\u{7}b", "ab")]
#[case::newline_and_tab("a\nb\tc", "abc")]
#[case::c1_control("a\u{9b}b", "ab")]
#[case::delete("a\u{7f}b", "ab")]
#[case::zero_width_space("a\u{200b}b", "ab")]
#[case::bidi_override("a\u{202e}b", "ab")]
#[case::line_separator("a\u{2028}b", "ab")]
#[case::paragraph_separator("a\u{2029}b", "ab")]
#[case::no_break_space("a\u{a0}b", "ab")]
#[case::ideographic_space("a\u{3000}b", "ab")]
#[case::private_use("a\u{e000}b", "ab")]
#[case::unassigned("a\u{378}b", "ab")]
#[case::keeps_cjk("\u{30e2}\u{30c7}\u{30eb}", "\u{30e2}\u{30c7}\u{30eb}")]
#[case::keeps_combining_mark("e\u{301}", "e\u{301}")]
#[case::keeps_emoji("a\u{1f600}b", "a\u{1f600}b")]
#[case::empty("", "")]
fn printable_drops_what_a_terminal_would_act_on(#[case] raw: &str, #[case] expected: &str) {
    assert_eq!(printable(raw), expected);
}
