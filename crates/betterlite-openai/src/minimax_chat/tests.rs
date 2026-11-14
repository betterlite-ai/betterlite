#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    DEFAULT_API_BASE, KEEPS_CACHE_CONTROL, complete_url, provider_info, resolve_base,
    supported_params,
};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn unrelated_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

#[test]
fn a_reasoning_model_gains_reasoning_split_and_thinking_after_the_openai_base_list() {
    let params = supported_params("MiniMax-M2", &config(), &unrelated_catalog(), true);
    let base = config().supported_params("MiniMax-M2", &unrelated_catalog());
    let mut expected = base.clone();
    expected.extend(["reasoning_split", "thinking"]);
    assert_eq!(params, expected);
}

#[test]
fn a_model_that_does_not_reason_gains_only_reasoning_split() {
    let params = supported_params("MiniMax-M2", &config(), &unrelated_catalog(), false);
    let base = config().supported_params("MiniMax-M2", &unrelated_catalog());
    let mut expected = base.clone();
    expected.push("reasoning_split");
    assert_eq!(params, expected);
}

#[test]
fn the_two_added_names_go_after_the_whole_base_list() {
    let params = supported_params("MiniMax-M2", &config(), &unrelated_catalog(), true);
    let mut tail = params.iter().rev();
    assert_eq!(tail.next(), Some(&"thinking"));
    assert_eq!(tail.next(), Some(&"reasoning_split"));
    assert_eq!(params.iter().filter(|name| **name == "thinking").count(), 1);
}

#[test]
fn minimax_keeps_cache_control_on_messages_and_tools() {
    assert!(KEEPS_CACHE_CONTROL);
}

#[rstest]
#[case::explicit_wins(
    Some("https://explicit.test/v1"),
    Some("https://env.test/v1"),
    "https://explicit.test/v1"
)]
#[case::env_wins_over_default(None, Some("https://env.test/v1"), "https://env.test/v1")]
#[case::default(None, None, DEFAULT_API_BASE)]
#[case::empty_explicit_falls_to_env(Some(""), Some("https://env.test/v1"), "https://env.test/v1")]
#[case::empty_env_falls_to_default(None, Some(""), DEFAULT_API_BASE)]
#[case::both_empty_is_the_default(Some(""), Some(""), DEFAULT_API_BASE)]
fn the_base_is_explicit_then_environment_then_default(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(resolve_base(api_base, env_base), expected);
}

#[rstest]
#[case::default_base(None, None, "https://api.minimax.io/v1/chat/completions")]
#[case::v1_base(
    Some("https://api.minimax.io/v1"),
    None,
    "https://api.minimax.io/v1/chat/completions"
)]
#[case::trailing_slash(
    Some("https://host.test/"),
    None,
    "https://host.test/v1/chat/completions"
)]
#[case::bare_host(
    Some("https://host.test"),
    None,
    "https://host.test/v1/chat/completions"
)]
#[case::complete_url(
    Some("https://host.test/v1/chat/completions"),
    None,
    "https://host.test/v1/chat/completions"
)]
#[case::complete_url_other_path(
    Some("https://host.test/custom/chat/completions"),
    None,
    "https://host.test/custom/chat/completions"
)]
#[case::environment_base(
    None,
    Some("https://env.test/"),
    "https://env.test/v1/chat/completions"
)]
#[case::china_base(
    Some("https://api.minimaxi.com/v1"),
    None,
    "https://api.minimaxi.com/v1/chat/completions"
)]
fn the_complete_url_rule_covers_every_base_shape(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(complete_url(api_base, env_base), expected);
}

#[test]
fn the_key_chain_ends_with_the_global_key() {
    let global = key("global-key");
    let (_, chosen) = provider_info(None, None, None, None, Some(&global));
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("global-key".to_owned())
    );
}

#[test]
fn an_explicit_key_wins_over_both_later_steps() {
    let explicit = key("explicit-key");
    let environment = key("env-key");
    let global = key("global-key");
    let (_, chosen) = provider_info(
        None,
        None,
        Some(&explicit),
        Some(&environment),
        Some(&global),
    );
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("explicit-key".to_owned())
    );
}

#[test]
fn the_environment_key_wins_over_the_global_key() {
    let environment = key("env-key");
    let global = key("global-key");
    let (_, chosen) = provider_info(None, None, None, Some(&environment), Some(&global));
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("env-key".to_owned())
    );
}

#[test]
fn an_empty_key_counts_as_unset_at_every_step() {
    let explicit = key("");
    let environment = key("");
    let global = key("global-key");
    let (_, chosen) = provider_info(
        None,
        None,
        Some(&explicit),
        Some(&environment),
        Some(&global),
    );
    assert_eq!(
        chosen.map(|key| key.expose_secret().clone()),
        Some("global-key".to_owned())
    );
}

#[test]
fn no_key_at_any_step_leaves_the_key_absent() {
    let (base, chosen) = provider_info(Some("https://host.test"), None, None, None, None);
    assert_eq!(base, "https://host.test");
    assert!(chosen.is_none());
}
