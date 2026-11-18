#![cfg(test)]

use secrecy::ExposeSecret as _;

use super::*;

fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn explicit_base_wins_and_the_suffix_is_appended() {
    let resolved = resolve_base(
        Surface::Messages,
        Some("https://explicit.test"),
        &Globals::default(),
        &environment(&[(API_BASE_VARIABLE, "https://env.test")]),
        false,
    );
    assert_eq!(resolved.url, "https://explicit.test/v1/messages");
    assert_eq!(resolved.notice, None);
}

#[test]
fn a_base_that_already_ends_with_the_suffix_keeps_one() {
    let resolved = resolve_base(
        Surface::Messages,
        Some("https://explicit.test/v1/messages"),
        &Globals::default(),
        &Environment::new(),
        false,
    );
    assert_eq!(resolved.url, "https://explicit.test/v1/messages");
}

#[test]
fn a_slash_ended_host_still_receives_the_suffix() {
    let resolved = resolve_base(
        Surface::Legacy,
        Some("https://explicit.test/"),
        &Globals::default(),
        &Environment::new(),
        false,
    );
    assert_eq!(resolved.url, "https://explicit.test//v1/complete");
}

#[test]
fn disabling_the_suffix_keeps_the_url_and_names_the_notice() {
    let resolved = resolve_base(
        Surface::Legacy,
        Some("https://explicit.test"),
        &Globals::default(),
        &Environment::new(),
        true,
    );
    assert_eq!(resolved.url, "https://explicit.test");
    assert_eq!(
        resolved.notice.as_deref(),
        Some("LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX is set, skipping /v1/complete suffix")
    );
}

#[test]
fn empty_environment_values_fall_through_to_the_default() {
    let resolved = resolve_base(
        Surface::Messages,
        None,
        &Globals::default(),
        &environment(&[(API_BASE_VARIABLE, ""), (BASE_URL_VARIABLE, "")]),
        false,
    );
    assert_eq!(resolved.url, "https://api.anthropic.com/v1/messages");
}

#[test]
fn the_key_chain_ends_at_the_raw_variable_even_when_empty() {
    let key = resolve_key(
        None,
        &Globals::default(),
        &environment(&[(API_KEY_VARIABLE, "")]),
    )
    .expect("the raw variable keeps its empty value");
    assert_eq!(key.expose_secret(), "");
}

#[test]
fn a_configured_key_beats_the_raw_variable() {
    let explicit = SecretString::from("sk-explicit");
    let key = resolve_key(
        Some(&explicit),
        &Globals::default(),
        &environment(&[(API_KEY_VARIABLE, "sk-env")]),
    )
    .expect("the explicit key");
    assert_eq!(key.expose_secret(), "sk-explicit");
    let globals = Globals {
        api_base: None,
        anthropic_key: Some(SecretString::from("sk-global")),
        api_key: None,
    };
    let key = resolve_key(
        None,
        &globals,
        &environment(&[(API_KEY_VARIABLE, "sk-env")]),
    )
    .expect("the global key");
    assert_eq!(key.expose_secret(), "sk-global");
}
