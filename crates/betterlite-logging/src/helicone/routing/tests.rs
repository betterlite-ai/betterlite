#![cfg(test)]

use rstest::rstest;

use super::{RoutingError, destination};
use crate::helicone::model_fallback::resolve;

#[rstest]
#[case::openai(
    "gpt-4o",
    "",
    "https://api.hconeai.com/oai/v1/log",
    "https://api.openai.com/v1"
)]
#[case::fallback_model_is_openai(
    "mistral-large",
    "",
    "https://api.hconeai.com/oai/v1/log",
    "https://api.openai.com/v1"
)]
#[case::command_is_openai(
    "command-r",
    "",
    "https://api.hconeai.com/oai/v1/log",
    "https://api.openai.com/v1"
)]
#[case::claude(
    "claude-3-opus",
    "anthropic",
    "https://api.hconeai.com/anthropic/v1/log",
    "https://api.anthropic.com/v1/messages"
)]
#[case::gemini(
    "gemini-pro",
    "gemini",
    "https://api.hconeai.com/custom/v1/log",
    "https://generativelanguage.googleapis.com/v1beta"
)]
#[case::vertex(
    "vertex_ai/gemini-pro",
    "",
    "https://api.hconeai.com/custom/v1/log",
    "https://aiplatform.googleapis.com/v1"
)]
#[case::vertex_beats_claude(
    "claude-3",
    "vertex_ai",
    "https://api.hconeai.com/custom/v1/log",
    "https://aiplatform.googleapis.com/v1"
)]
#[case::vertex_prefix_beats_claude(
    "vertex_ai/claude-3",
    "",
    "https://api.hconeai.com/custom/v1/log",
    "https://aiplatform.googleapis.com/v1"
)]
#[case::vertex_beats_gemini(
    "gemini-pro",
    "vertex_ai",
    "https://api.hconeai.com/custom/v1/log",
    "https://aiplatform.googleapis.com/v1"
)]
fn routing_picks_the_endpoint_and_provider_url(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] log_url: &str,
    #[case] provider_url: &str,
) {
    let chosen = destination("https://api.hconeai.com", &resolve(model, provider)).unwrap();
    assert_eq!(chosen.log_url.as_str(), log_url);
    assert_eq!(chosen.provider_url, provider_url);
}

#[test]
fn routing_builds_the_url_on_the_configured_base() {
    let chosen = destination("http://127.0.0.1:8787", &resolve("gpt-4", "")).unwrap();
    assert_eq!(chosen.log_url.as_str(), "http://127.0.0.1:8787/oai/v1/log");
}

#[test]
fn routing_rejects_a_base_that_is_not_a_url() {
    assert!(matches!(
        destination("not a url", &resolve("gpt-4", "")),
        Err(RoutingError::InvalidUrl(_))
    ));
}
